//! Live topology graph derived from the watchers' objects (spec 036 R11, FR-018).
//!
//! Incremental: every upsert/remove recomputes only the edges incident to that object and
//! returns a [`ScopedDelta`] annotated with namespaces so the per-user filter can drop nodes and
//! edges the caller may not see (never placeholders).

use crate::authz::namespace_visible;
use fathom::protocol::v1::{
    EdgeDelta, TopologyDelta, TopologyEdge, TopologyHealth, TopologyNode, TopologyRelation,
    VisibleScope,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Minimal projection of a watched object.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj {
    pub uid: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub health: TopologyHealth,
    /// Owner reference uids.
    pub owners: Vec<String>,
    pub labels: BTreeMap<String, String>,
    /// Service selector (`Some` only for Services).
    pub selector: Option<BTreeMap<String, String>>,
    /// Pod `spec.nodeName`.
    pub node_name: Option<String>,
    /// HPA `scaleTargetRef` as (kind, name).
    pub scale_target: Option<(String, String)>,
}

impl Obj {
    fn node(&self) -> TopologyNode {
        TopologyNode {
            uid: self.uid.clone(),
            kind: self.kind.clone(),
            namespace: self.namespace.clone(),
            name: self.name.clone(),
            health: self.health,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Rel {
    Owns,
    Selects,
    ScheduledOn,
    Scales,
}

impl From<Rel> for TopologyRelation {
    fn from(r: Rel) -> Self {
        match r {
            Rel::Owns => TopologyRelation::Owns,
            Rel::Selects => TopologyRelation::Selects,
            Rel::ScheduledOn => TopologyRelation::ScheduledOn,
            Rel::Scales => TopologyRelation::Scales,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EKey {
    from: String,
    to: String,
    rel: Rel,
}

impl EKey {
    fn edge(&self) -> TopologyEdge {
        TopologyEdge {
            from: self.from.clone(),
            to: self.to.clone(),
            relation: self.rel.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedEdge {
    pub edge: TopologyEdge,
    /// `None` = cluster-scoped endpoint.
    pub from_ns: Option<String>,
    pub to_ns: Option<String>,
}

/// A graph change annotated with namespaces; sequence is assigned by the event log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopedDelta {
    pub added: Vec<TopologyNode>,
    /// (uid, namespace) of removed nodes.
    pub removed: Vec<(String, Option<String>)>,
    pub edges_added: Vec<ScopedEdge>,
    pub edges_removed: Vec<ScopedEdge>,
}

impl ScopedDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.edges_added.is_empty()
            && self.edges_removed.is_empty()
    }

    /// Filter to the caller's scope; `None` when nothing remains visible.
    pub fn filter(&self, scope: &VisibleScope, seq: u64) -> Option<TopologyDelta> {
        let vis = |ns: &Option<String>| namespace_visible(scope, ns.as_deref());
        let edge_ok = |e: &ScopedEdge| vis(&e.from_ns) && vis(&e.to_ns);
        let out = TopologyDelta {
            seq,
            added: self
                .added
                .iter()
                .filter(|n| namespace_visible(scope, n.namespace.as_deref()))
                .cloned()
                .collect(),
            removed: self
                .removed
                .iter()
                .filter(|(_, ns)| vis(ns))
                .map(|(u, _)| u.clone())
                .collect(),
            edges: EdgeDelta {
                added: self
                    .edges_added
                    .iter()
                    .filter(|e| edge_ok(e))
                    .map(|e| e.edge.clone())
                    .collect(),
                removed: self
                    .edges_removed
                    .iter()
                    .filter(|e| edge_ok(e))
                    .map(|e| e.edge.clone())
                    .collect(),
            },
        };
        let empty = out.added.is_empty()
            && out.removed.is_empty()
            && out.edges.added.is_empty()
            && out.edges.removed.is_empty();
        (!empty).then_some(out)
    }
}

#[derive(Default)]
pub struct TopologyState {
    objs: HashMap<String, Obj>,
    edges_by_node: HashMap<String, HashSet<EKey>>,
    children_by_owner: HashMap<String, HashSet<String>>,
    pods_by_ns: HashMap<String, HashSet<String>>,
    svcs_by_ns: HashMap<String, HashSet<String>>,
    hpas_by_ns: HashMap<String, HashSet<String>>,
    pods_by_node: HashMap<String, HashSet<String>>,
    node_by_name: HashMap<String, String>,
    by_key: HashMap<(String, Option<String>, String), String>,
}

fn selects(selector: &BTreeMap<String, String>, labels: &BTreeMap<String, String>) -> bool {
    !selector.is_empty() && selector.iter().all(|(k, v)| labels.get(k) == Some(v))
}

fn set_of<'a>(m: &'a mut HashMap<String, HashSet<String>>, k: &str) -> &'a mut HashSet<String> {
    m.entry(k.to_string()).or_default()
}

impl TopologyState {
    pub fn len(&self) -> usize {
        self.objs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }

    /// Namespace of a known object: outer `None` = unknown uid, inner `None` = cluster-scoped.
    pub fn ns_of(&self, uid: &str) -> Option<Option<String>> {
        self.objs.get(uid).map(|o| o.namespace.clone())
    }

    pub fn uid_of(&self, kind: &str, ns: Option<&str>, name: &str) -> Option<String> {
        self.by_key
            .get(&(kind.to_string(), ns.map(str::to_string), name.to_string()))
            .cloned()
    }

    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.objs.values().filter_map(|o| o.namespace.as_deref())
    }

    fn unindex(&mut self, o: &Obj) {
        for ow in &o.owners {
            if let Some(s) = self.children_by_owner.get_mut(ow) {
                s.remove(&o.uid);
            }
        }
        let ns = o.namespace.clone().unwrap_or_default();
        match o.kind.as_str() {
            "Pod" => {
                set_of(&mut self.pods_by_ns, &ns).remove(&o.uid);
                if let Some(n) = &o.node_name {
                    set_of(&mut self.pods_by_node, n).remove(&o.uid);
                }
            }
            "Service" => {
                set_of(&mut self.svcs_by_ns, &ns).remove(&o.uid);
            }
            "HorizontalPodAutoscaler" => {
                set_of(&mut self.hpas_by_ns, &ns).remove(&o.uid);
            }
            "Node" => {
                self.node_by_name.remove(&o.name);
            }
            _ => {}
        }
        self.by_key
            .remove(&(o.kind.clone(), o.namespace.clone(), o.name.clone()));
    }

    fn index(&mut self, o: &Obj) {
        for ow in &o.owners {
            set_of(&mut self.children_by_owner, ow).insert(o.uid.clone());
        }
        let ns = o.namespace.clone().unwrap_or_default();
        match o.kind.as_str() {
            "Pod" => {
                set_of(&mut self.pods_by_ns, &ns).insert(o.uid.clone());
                if let Some(n) = &o.node_name {
                    set_of(&mut self.pods_by_node, n).insert(o.uid.clone());
                }
            }
            "Service" => {
                set_of(&mut self.svcs_by_ns, &ns).insert(o.uid.clone());
            }
            "HorizontalPodAutoscaler" => {
                set_of(&mut self.hpas_by_ns, &ns).insert(o.uid.clone());
            }
            "Node" => {
                self.node_by_name.insert(o.name.clone(), o.uid.clone());
            }
            _ => {}
        }
        self.by_key.insert(
            (o.kind.clone(), o.namespace.clone(), o.name.clone()),
            o.uid.clone(),
        );
    }

    fn desired_edges(&self, o: &Obj) -> HashSet<EKey> {
        let mut out = HashSet::new();
        let mut add = |from: &str, to: &str, rel| {
            out.insert(EKey {
                from: from.into(),
                to: to.into(),
                rel,
            });
        };
        for ow in &o.owners {
            if self.objs.contains_key(ow) || *ow == o.uid {
                add(ow, &o.uid, Rel::Owns);
            }
        }
        for c in self.children_by_owner.get(&o.uid).into_iter().flatten() {
            if self.objs.contains_key(c) || *c == o.uid {
                add(&o.uid, c, Rel::Owns);
            }
        }
        let ns = o.namespace.clone().unwrap_or_default();
        match o.kind.as_str() {
            "Pod" => {
                if let Some(n) = o.node_name.as_ref().and_then(|n| self.node_by_name.get(n)) {
                    add(&o.uid, n, Rel::ScheduledOn);
                }
                for s in self.svcs_by_ns.get(&ns).into_iter().flatten() {
                    let svc = &self.objs[s];
                    if svc
                        .selector
                        .as_ref()
                        .is_some_and(|sel| selects(sel, &o.labels))
                    {
                        add(s, &o.uid, Rel::Selects);
                    }
                }
            }
            "Node" => {
                for p in self.pods_by_node.get(&o.name).into_iter().flatten() {
                    add(p, &o.uid, Rel::ScheduledOn);
                }
            }
            "Service" => {
                if let Some(sel) = o.selector.as_ref() {
                    for p in self.pods_by_ns.get(&ns).into_iter().flatten() {
                        if selects(sel, &self.objs[p].labels) {
                            add(&o.uid, p, Rel::Selects);
                        }
                    }
                }
            }
            "HorizontalPodAutoscaler" => {
                if let Some((k, n)) = &o.scale_target
                    && let Some(t) = self
                        .by_key
                        .get(&(k.clone(), o.namespace.clone(), n.clone()))
                {
                    add(&o.uid, t, Rel::Scales);
                }
            }
            _ => {
                for h in self.hpas_by_ns.get(&ns).into_iter().flatten() {
                    if let Some((k, n)) = &self.objs[h].scale_target
                        && *k == o.kind
                        && *n == o.name
                    {
                        add(h, &o.uid, Rel::Scales);
                    }
                }
            }
        }
        out
    }

    fn scoped(&self, k: &EKey, extra: Option<&Obj>) -> ScopedEdge {
        let ns = |u: &str| {
            self.objs
                .get(u)
                .map(|o| o.namespace.clone())
                .or_else(|| extra.filter(|e| e.uid == u).map(|e| e.namespace.clone()))
                .flatten()
        };
        ScopedEdge {
            edge: k.edge(),
            from_ns: ns(&k.from),
            to_ns: ns(&k.to),
        }
    }

    pub fn upsert(&mut self, o: Obj) -> ScopedDelta {
        let mut delta = ScopedDelta::default();
        let prev = self.objs.remove(&o.uid);
        if let Some(p) = &prev {
            self.unindex(p);
        }
        self.index(&o);
        let node_changed = prev.as_ref().map(Obj::node) != Some(o.node());
        self.objs.insert(o.uid.clone(), o.clone());
        if node_changed {
            delta.added.push(o.node());
        }
        let new = self.desired_edges(&o);
        let old = self.edges_by_node.get(&o.uid).cloned().unwrap_or_default();
        for k in new.difference(&old) {
            delta.edges_added.push(self.scoped(k, None));
        }
        for k in old.difference(&new) {
            delta.edges_removed.push(self.scoped(k, None));
        }
        for k in old.difference(&new) {
            self.edges_by_node
                .entry(k.from.clone())
                .or_default()
                .remove(k);
            self.edges_by_node
                .entry(k.to.clone())
                .or_default()
                .remove(k);
        }
        for k in new.difference(&old) {
            self.edges_by_node
                .entry(k.from.clone())
                .or_default()
                .insert(k.clone());
            self.edges_by_node
                .entry(k.to.clone())
                .or_default()
                .insert(k.clone());
        }
        delta
    }

    pub fn remove(&mut self, uid: &str) -> ScopedDelta {
        let mut delta = ScopedDelta::default();
        let Some(o) = self.objs.get(uid).cloned() else {
            return delta;
        };
        let old = self.edges_by_node.remove(uid).unwrap_or_default();
        for k in &old {
            delta.edges_removed.push(self.scoped(k, None));
            let other = if k.from == uid { &k.to } else { &k.from };
            if let Some(s) = self.edges_by_node.get_mut(other) {
                s.remove(k);
            }
        }
        self.unindex(&o);
        self.objs.remove(uid);
        delta.removed.push((uid.to_string(), o.namespace));
        delta
    }

    /// Snapshot filtered to the caller's scope (nodes and edges with both endpoints visible).
    pub fn snapshot(&self, scope: &VisibleScope) -> (Vec<TopologyNode>, Vec<TopologyEdge>) {
        let vis = |u: &str| {
            self.objs
                .get(u)
                .is_some_and(|o| namespace_visible(scope, o.namespace.as_deref()))
        };
        let mut nodes: Vec<TopologyNode> = self
            .objs
            .values()
            .filter(|o| namespace_visible(scope, o.namespace.as_deref()))
            .map(Obj::node)
            .collect();
        nodes.sort_by(|a, b| a.uid.cmp(&b.uid));
        let mut seen = HashSet::new();
        let mut edges = Vec::new();
        for set in self.edges_by_node.values() {
            for k in set {
                if vis(&k.from) && vis(&k.to) && seen.insert(k.clone()) {
                    edges.push(k.edge());
                }
            }
        }
        edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        (nodes, edges)
    }
}

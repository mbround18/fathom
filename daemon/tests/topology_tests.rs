//! T045: topology derivation, deltas and per-user edge dropping.

use fathom::protocol::v1::{TopologyHealth, TopologyRelation, VisibleScope};
use fathom_daemon::topology::{Obj, TopologyState};
use std::collections::BTreeMap;

fn obj(uid: &str, kind: &str, ns: Option<&str>, name: &str) -> Obj {
    Obj {
        uid: uid.into(),
        kind: kind.into(),
        namespace: ns.map(str::to_string),
        name: name.into(),
        health: TopologyHealth::Ok,
        owners: vec![],
        labels: BTreeMap::new(),
        selector: None,
        node_name: None,
        scale_target: None,
    }
}

fn labels(p: &[(&str, &str)]) -> BTreeMap<String, String> {
    p.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn scope(ns: &[&str], cluster: bool) -> VisibleScope {
    let mut n: Vec<String> = ns.iter().map(|s| s.to_string()).collect();
    n.sort();
    VisibleScope {
        identity_key: "k".into(),
        namespaces: n,
        cluster_scoped: cluster,
        resolved_at: "t".into(),
    }
}

fn rels(d: &fathom_daemon::topology::ScopedDelta) -> Vec<(String, String, TopologyRelation)> {
    let mut v: Vec<_> = d
        .edges_added
        .iter()
        .map(|e| (e.edge.from.clone(), e.edge.to.clone(), e.edge.relation))
        .collect();
    v.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    v
}

#[test]
fn owner_chain_edges_regardless_of_arrival_order() {
    let mut t = TopologyState::default();
    let mut pod = obj("p", "Pod", Some("a"), "p");
    pod.owners = vec!["rs".into()];
    let mut rs = obj("rs", "ReplicaSet", Some("a"), "rs");
    rs.owners = vec!["d".into()];
    // child first, parents later
    let d1 = t.upsert(pod);
    assert!(d1.edges_added.is_empty(), "owner not known yet");
    let d2 = t.upsert(rs);
    assert_eq!(
        rels(&d2),
        vec![("rs".into(), "p".into(), TopologyRelation::Owns)]
    );
    let d3 = t.upsert(obj("d", "Deployment", Some("a"), "d"));
    assert_eq!(
        rels(&d3),
        vec![("d".into(), "rs".into(), TopologyRelation::Owns)]
    );
    let (nodes, edges) = t.snapshot(&scope(&["a"], false));
    assert_eq!(nodes.len(), 3);
    assert_eq!(edges.len(), 2);
}

#[test]
fn service_selector_pod_node_and_hpa_edges() {
    let mut t = TopologyState::default();
    let mut pod = obj("p", "Pod", Some("a"), "p");
    pod.labels = labels(&[("app", "web")]);
    pod.node_name = Some("n1".into());
    t.upsert(pod);
    let mut svc = obj("s", "Service", Some("a"), "s");
    svc.selector = Some(labels(&[("app", "web")]));
    let d = t.upsert(svc);
    assert_eq!(
        rels(&d),
        vec![("s".into(), "p".into(), TopologyRelation::Selects)]
    );
    let d = t.upsert(obj("n1", "Node", None, "n1"));
    assert_eq!(
        rels(&d),
        vec![("p".into(), "n1".into(), TopologyRelation::ScheduledOn)]
    );
    let mut hpa = obj("h", "HorizontalPodAutoscaler", Some("a"), "h");
    hpa.scale_target = Some(("Deployment".into(), "web".into()));
    assert!(t.upsert(hpa).edges_added.is_empty());
    let d = t.upsert(obj("d", "Deployment", Some("a"), "web"));
    assert_eq!(
        rels(&d),
        vec![("h".into(), "d".into(), TopologyRelation::Scales)]
    );
}

#[test]
fn service_with_empty_selector_selects_nothing() {
    let mut t = TopologyState::default();
    t.upsert(obj("p", "Pod", Some("a"), "p"));
    let mut svc = obj("s", "Service", Some("a"), "s");
    svc.selector = Some(BTreeMap::new());
    assert!(t.upsert(svc).edges_added.is_empty());
}

#[test]
fn label_change_moves_selector_edge_and_health_change_is_a_delta() {
    let mut t = TopologyState::default();
    let mut svc = obj("s", "Service", Some("a"), "s");
    svc.selector = Some(labels(&[("app", "web")]));
    t.upsert(svc);
    let mut pod = obj("p", "Pod", Some("a"), "p");
    pod.labels = labels(&[("app", "web")]);
    assert_eq!(t.upsert(pod.clone()).edges_added.len(), 1);
    // unchanged -> empty delta
    assert!(t.upsert(pod.clone()).is_empty());
    pod.labels = labels(&[("app", "other")]);
    let d = t.upsert(pod.clone());
    assert_eq!(d.edges_removed.len(), 1);
    pod.health = TopologyHealth::Error;
    let d = t.upsert(pod);
    assert_eq!(d.added.len(), 1);
    assert_eq!(d.added[0].health, TopologyHealth::Error);
}

#[test]
fn remove_emits_node_and_incident_edges() {
    let mut t = TopologyState::default();
    let mut pod = obj("p", "Pod", Some("a"), "p");
    pod.owners = vec!["rs".into()];
    t.upsert(pod);
    t.upsert(obj("rs", "ReplicaSet", Some("a"), "rs"));
    let d = t.remove("rs");
    assert_eq!(d.removed, vec![("rs".to_string(), Some("a".to_string()))]);
    assert_eq!(d.edges_removed.len(), 1);
    assert!(t.remove("rs").is_empty());
    let (nodes, edges) = t.snapshot(&scope(&["a"], false));
    assert_eq!((nodes.len(), edges.len()), (1, 0));
}

#[test]
fn per_user_filtering_drops_edges_without_placeholders() {
    let mut t = TopologyState::default();
    let mut pod = obj("p", "Pod", Some("a"), "p");
    pod.node_name = Some("n1".into());
    t.upsert(pod);
    t.upsert(obj("n1", "Node", None, "n1"));
    let mut other = obj("q", "Pod", Some("b"), "q");
    other.node_name = Some("n1".into());
    t.upsert(other);

    // ns a only, no cluster: sees pod p, but no node and no edge to it
    let (nodes, edges) = t.snapshot(&scope(&["a"], false));
    assert_eq!(
        nodes.iter().map(|n| n.uid.as_str()).collect::<Vec<_>>(),
        vec!["p"]
    );
    assert!(edges.is_empty());
    // cluster + a: node visible, edge p->n1 visible, q hidden and its edge dropped
    let (nodes, edges) = t.snapshot(&scope(&["a"], true));
    assert_eq!(nodes.len(), 2);
    assert_eq!(edges.len(), 1);
    // nothing
    let (nodes, edges) = t.snapshot(&scope(&[], false));
    assert!(nodes.is_empty() && edges.is_empty());
}

#[test]
fn deltas_are_filtered_per_user_and_carry_seq() {
    let mut t = TopologyState::default();
    t.upsert(obj("n1", "Node", None, "n1"));
    let mut pod = obj("p", "Pod", Some("a"), "p");
    pod.node_name = Some("n1".into());
    let d = t.upsert(pod);
    let f = d.filter(&scope(&["a"], false), 7).expect("pod visible");
    assert_eq!(f.seq, 7);
    assert_eq!(f.added.len(), 1);
    assert!(f.edges.added.is_empty(), "edge to hidden node dropped");
    let f = d.filter(&scope(&["a"], true), 8).unwrap();
    assert_eq!(f.edges.added.len(), 1);
    assert!(d.filter(&scope(&["zzz"], false), 9).is_none());
}

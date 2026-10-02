//! 046-fleet-model: the fleet as a named, labelled, groupable set of kubeconfig contexts.
//!
//! This family owns the fleet *model* — the operator-authored metadata in
//! `~/.fathom/fleet.json` — and nothing else. It is deliberately the quietest family in the
//! backend: **no command here starts a poller, builds a client or opens a watch** (FR-015).
//! Fleet metadata is read at launch and on explicit refresh, and written on an operator
//! edit. That is what lets SC-005 claim zero additional idle CPU and zero additional API
//! server requests; 027's `FleetState` poller remains the only thing that talks to clusters.
//!
//! Group membership is computed, never stored (FR-004): the file holds selector expressions
//! verbatim as typed, and membership is derived from the members' current labels every time
//! the model is emitted.

use std::collections::{BTreeMap, BTreeSet};

use crossbeam_channel::Sender;
use kube::core::Selector;

use super::{events, persistence};
use crate::backend::fleet_model;
use crate::backend::persistence::{FleetFile, FleetLoad};
use crate::backend::state::BackendState;
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, FleetGroup, FleetMember,
    FleetMemberAvailability, Route, SystemEvent, section_keys,
};

/// Handles every 046 command; returns the command back if it belongs to another family.
pub(super) fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshFleetModel => refresh_fleet_model(state, sender),
        other => return Ok(Some(other)),
    }
    Ok(None)
}

/// Every event in this family is routed globally on the `system` section, matching
/// `FleetSummaryUpdated`/`ContextsUpdated`: the model describes the whole fleet, not the
/// connected context.
fn emit(state: &BackendState, sender: &Sender<BackendEventEnvelope>, event: SystemEvent) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(event),
    );
}

/// `RefreshFleetModel`: read the file, reconcile availability against the kubeconfig, and
/// emit the model. Reads one file and one kubeconfig; opens no connection (FR-015).
///
/// A load failure is reported *and* the defaults are still emitted, so the UI is never left
/// with no model at all — an operator with an unparseable file sees an empty fleet and the
/// parser's own reason, rather than a view that never resolves.
fn refresh_fleet_model(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    let path = persistence::fleet_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.fathom/fleet.json".to_owned());
    let FleetLoad { file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed { path, reason },
        );
    }
    let contexts = kubeconfig_contexts(state);
    let (members, groups) = model_from(&file, &contexts);
    log_group_membership(&members, &groups);
    emit(
        state,
        sender,
        SystemEvent::FleetModelUpdated { members, groups },
    );
}

/// The contexts the fleet reconciles against: the kubeconfig's own list, as 027's poller
/// reads it. A backend with no local features (031's hosted server) has no kubeconfig at
/// all — its only context is the access's fixed home context, exactly as
/// `kube::send_contexts_for` decides it, so a hosted fleet does not read every member as
/// `Absent`.
fn kubeconfig_contexts(state: &BackendState) -> Vec<String> {
    if !state.access.allows_local_features() {
        return state.access.home_context().into_iter().collect();
    }
    crate::backend::fleet::all_kubeconfig_contexts()
}

/// Build the emitted model from the persisted file and the current context list.
///
/// The member set is the **union** of the two: a member in the file but not in the
/// kubeconfig is `Absent` with every byte of its metadata intact, never dropped (FR-009),
/// and a context in the kubeconfig with no entry in the file is a `Present` member with no
/// display name and no labels. Both are keyed by context name, so the result is sorted and
/// stable.
fn model_from(file: &FleetFile, contexts: &[String]) -> (Vec<FleetMember>, Vec<FleetGroup>) {
    let present: BTreeSet<&str> = contexts.iter().map(String::as_str).collect();
    let names: BTreeSet<&str> = file
        .members
        .keys()
        .map(String::as_str)
        .chain(present.iter().copied())
        .collect();

    let members = names
        .into_iter()
        .map(|context| {
            let record = file.members.get(context);
            FleetMember {
                context: context.to_owned(),
                display_name: record.and_then(|r| r.display_name.clone()),
                labels: record.map(|r| r.labels.clone()).unwrap_or_default(),
                origins: record.map(|r| r.origins.clone()).unwrap_or_default(),
                availability: if present.contains(context) {
                    FleetMemberAvailability::Present
                } else {
                    FleetMemberAvailability::Absent
                },
            }
        })
        .collect();

    let groups = file
        .groups
        .iter()
        .map(|(name, record)| FleetGroup {
            name: name.clone(),
            selector: record.selector.clone(),
            origin: record.origin,
        })
        .collect();

    (members, groups)
}

/// Compute each member's group membership (FR-004) and record it in the log.
///
/// Membership is not carried in `FleetModelUpdated`: the UI derives it from the same labels
/// and selectors the event already contains, so there is one definition of it rather than
/// two. What this pass is for is the backend's own check that every *stored* selector still
/// parses — a group whose expression no longer parses would otherwise show as a group
/// matching nothing, which is indistinguishable from a group whose selector matches nothing
/// (FR-005). Here it says so by name instead.
fn log_group_membership(members: &[FleetMember], groups: &[FleetGroup]) {
    let mut parsed: Vec<(&str, Selector)> = Vec::with_capacity(groups.len());
    for group in groups {
        match fleet_model::parse_selector(&group.selector) {
            Ok(selector) => parsed.push((group.name.as_str(), selector)),
            Err(e) => tracing::warn!(
                "fleet group {:?} has an unparseable selector and matches nothing: {}",
                group.name,
                e
            ),
        }
    }
    if parsed.is_empty() {
        return;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for member in members {
        for name in fleet_model::groups_matching(
            parsed.iter().map(|(name, selector)| (*name, selector)),
            &member.labels,
        ) {
            *counts.entry(name).or_default() += 1;
        }
    }
    tracing::debug!(
        "fleet model: {} members, {} groups, membership {:?}",
        members.len(),
        groups.len(),
        counts
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::persistence::{FleetGroupRecord, FleetMemberRecord};
    use crate::protocol::SettingOrigin;

    fn file() -> FleetFile {
        FleetFile {
            members: BTreeMap::from([
                (
                    "prod-a".to_owned(),
                    FleetMemberRecord {
                        display_name: Some("prod a".to_owned()),
                        labels: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
                        origins: BTreeMap::from([(
                            "display_name".to_owned(),
                            SettingOrigin::Local,
                        )]),
                    },
                ),
                (
                    "retired".to_owned(),
                    FleetMemberRecord {
                        display_name: Some("the one we decommissioned".to_owned()),
                        labels: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
                        origins: BTreeMap::new(),
                    },
                ),
            ]),
            groups: BTreeMap::from([(
                "prod".to_owned(),
                FleetGroupRecord {
                    selector: "env in (prod,staging)".to_owned(),
                    origin: SettingOrigin::Local,
                },
            )]),
            ..FleetFile::default()
        }
    }

    /// FR-009: a member the kubeconfig no longer lists is `Absent` **with its metadata**,
    /// not a dropped row. Losing the name is how an operator loses the only record of which
    /// cluster an ARN was.
    #[test]
    fn a_member_missing_from_the_kubeconfig_is_absent_with_its_metadata() {
        let (members, groups) = model_from(&file(), &["prod-a".to_owned()]);
        let retired = members.iter().find(|m| m.context == "retired").unwrap();
        assert_eq!(retired.availability, FleetMemberAvailability::Absent);
        assert_eq!(
            retired.display_name.as_deref(),
            Some("the one we decommissioned")
        );
        assert_eq!(retired.labels["env"], "prod");
        // The group it still matches is untouched by its absence.
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].selector, "env in (prod,staging)");
    }

    /// A context Fathom can see but the file has never heard of is a member all the same —
    /// an unlabelled, unnamed, `Present` one. The fleet is the union, not the file.
    #[test]
    fn a_context_with_no_entry_in_the_file_is_a_bare_present_member() {
        let (members, _) = model_from(&file(), &["prod-a".to_owned(), "brand-new".to_owned()]);
        let fresh = members.iter().find(|m| m.context == "brand-new").unwrap();
        assert_eq!(fresh.availability, FleetMemberAvailability::Present);
        assert_eq!(fresh.display_name, None);
        assert!(fresh.labels.is_empty());
        assert!(fresh.origins.is_empty());
        // Sorted and deduplicated: prod-a appears once, from both sides.
        let names: Vec<&str> = members.iter().map(|m| m.context.as_str()).collect();
        assert_eq!(names, ["brand-new", "prod-a", "retired"]);
    }

    /// Membership is computed from labels and selectors, with no per-cluster filing step
    /// (FR-004) — and an absent member still matches, because its labels are still there.
    #[test]
    fn membership_is_computed_from_labels() {
        let (members, groups) = model_from(&file(), &["prod-a".to_owned()]);
        let selector = fleet_model::parse_selector(&groups[0].selector).unwrap();
        let matched: Vec<&str> = members
            .iter()
            .filter(|m| !fleet_model::groups_matching([("prod", &selector)], &m.labels).is_empty())
            .map(|m| m.context.as_str())
            .collect();
        assert_eq!(matched, ["prod-a", "retired"]);
    }

    /// Every command but `RefreshFleetModel` falls through to the next family, so the
    /// commands this family does not implement yet behave exactly as they did before it
    /// existed rather than being silently swallowed here.
    #[test]
    fn unclaimed_commands_are_handed_back() {
        let mut state = BackendState::new();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let back = handle(
            BackendCmd::ForgetFleetMember {
                context: "prod-a".to_owned(),
            },
            &mut state,
            &tx,
        )
        .unwrap();
        assert!(matches!(back, Some(BackendCmd::ForgetFleetMember { .. })));
    }
}

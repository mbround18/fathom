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
use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;
use kube::core::Selector;

use super::{events, persistence};
use crate::backend::fleet_model;
use crate::backend::idp_directory::FathomRole;
use crate::backend::persistence::{FleetFile, FleetGroupRecord, FleetLoad, FleetMemberRecord};
use crate::backend::state::BackendState;
use crate::backend::team_store::{self, StoreError};
use crate::protocol::{
    AdoptedFleetDefinition, BackendCmd, BackendEvent, BackendEventEnvelope, Correlation,
    FleetContextSummary, FleetCoverageGap, FleetGapReason, FleetGroup, FleetGroupRollup,
    FleetHealthState, FleetMember, FleetMemberAvailability, FleetMemberShare, Route, SettingOrigin,
    SharedFleetDefinition, SystemEvent, section_keys,
};

/// The maximum length of a display name and of a group name (`data-model.md`).
const MAX_NAME_CHARS: usize = 128;

/// Handles every 046 command; returns the command back if it belongs to another family.
pub(super) fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshFleetModel => refresh_fleet_model(state, sender),
        BackendCmd::SetFleetMemberDisplayName {
            context,
            display_name,
        } => {
            edit_model(state, sender, |file| {
                set_display_name(file, &context, display_name.as_deref())
            });
        }
        BackendCmd::SetFleetMemberLabels { context, labels } => {
            edit_model(state, sender, |file| set_labels(file, &context, labels));
        }
        BackendCmd::ForgetFleetMember { context } => {
            edit_model(state, sender, |file| forget_member(file, &context));
        }
        BackendCmd::SaveFleetGroup { name, selector } => {
            edit_model(state, sender, |file| save_group(file, &name, &selector));
        }
        BackendCmd::DeleteFleetGroup { name } => {
            edit_model(state, sender, |file| delete_group(file, &name));
        }
        BackendCmd::RequestFleetGroupRollup { group } => request_rollup(state, sender, &group),
        BackendCmd::RefreshSharedFleetDefinitions => refresh_shared_definitions(state, sender),
        BackendCmd::PublishFleetDefinition { id } => {
            if gate(state, sender, FathomRole::Editor) {
                publish_definition(state, sender, &id);
            }
        }
        BackendCmd::AdoptFleetDefinition { id } => adopt_definition(state, sender, &id),
        BackendCmd::RemoveFleetDefinition { id } => remove_definition(state, sender, &id),
        other => return Ok(Some(other)),
    }
    Ok(None)
}

/// A refused edit: the validator's or parser's own message, verbatim, and the field it names.
///
/// This exists so every editing command refuses the same way — load, validate *everything*,
/// then write — and so a rejection cannot be constructed without a reason (FR-003, FR-006).
#[derive(Debug)]
struct Rejection {
    reason: String,
    field: Option<String>,
}

impl Rejection {
    fn new(reason: impl Into<String>, field: &str) -> Self {
        Self {
            reason: reason.into(),
            field: Some(field.to_owned()),
        }
    }
}

/// The one write path: load the file, let `edit` mutate it or refuse, persist, emit the model.
///
/// Three properties the commands all inherit from it:
/// - **Nothing is persisted on a refusal.** `edit` mutates a copy that is dropped unread, and
///   `FleetModelRejected` is emitted instead of — never alongside — `FleetModelUpdated`.
/// - **An unparseable file is never written over** (Principle VI). A load failure reports itself
///   and stops: overwriting would turn one unreadable file into one *lost* file, and the operator
///   has had no chance to look at it yet.
/// - **No cluster is touched.** One file read, one kubeconfig read, one file write (FR-015).
///
/// Returns whether the edit was persisted, so a caller with more to emit afterwards (US3's
/// adoption, which also emits `SharedFleetDefinitionsUpdated`) can tell a write from a refusal
/// without re-deriving the outcome.
fn edit_model(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    edit: impl FnOnce(&mut FleetFile) -> Result<(), Rejection>,
) -> bool {
    let FleetLoad { mut file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
        return false;
    }
    if let Err(Rejection { reason, field }) = edit(&mut file) {
        emit(
            state,
            sender,
            SystemEvent::FleetModelRejected { reason, field },
        );
        return false;
    }
    if let Err(e) = persistence::save_fleet_model(&file) {
        // The edit was valid but the disk refused it, so nothing was persisted — which is
        // exactly what `FleetModelRejected` promises. `field` is `None`: no input is at fault.
        emit(
            state,
            sender,
            SystemEvent::FleetModelRejected {
                reason: format!("{e:#}"),
                field: None,
            },
        );
        return false;
    }
    emit_model(state, sender, &file);
    true
}

/// Emit the whole model as reconciled against the current kubeconfig.
fn emit_model(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, file: &FleetFile) {
    let contexts = kubeconfig_contexts(state);
    let (members, groups) = model_from(file, &contexts);
    emit(
        state,
        sender,
        SystemEvent::FleetModelUpdated { members, groups },
    );
}

fn fleet_path_display() -> String {
    persistence::fleet_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.fathom/fleet.json".to_owned())
}

/// A context name is a member's identity (FR-001); an empty one names nothing.
fn check_context(context: &str) -> Result<(), Rejection> {
    if context.trim().is_empty() {
        return Err(Rejection::new(
            "a member is identified by its kubeconfig context name, which must not be empty",
            "context",
        ));
    }
    Ok(())
}

/// T026 `SetFleetMemberDisplayName`: trim, and store an empty name as *absent*.
///
/// `Some("")` is not a display name, it is the absence of one rendered as a value — and FR-002
/// says the display name is shown beside the context, so an empty string would render as a blank
/// where a name should be. Clearing the name also clears its origin: there is no such thing as a
/// locally-set nothing.
fn set_display_name(
    file: &mut FleetFile,
    context: &str,
    display_name: Option<&str>,
) -> Result<(), Rejection> {
    check_context(context)?;
    let trimmed = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Some(name) = &trimmed
        && name.chars().count() > MAX_NAME_CHARS
    {
        return Err(Rejection::new(
            format!(
                "display name \"{name}\" is invalid: must be 1-{MAX_NAME_CHARS} characters, and it is {}",
                name.chars().count()
            ),
            "display_name",
        ));
    }
    let record = file.members.entry(context.to_owned()).or_default();
    match &trimmed {
        Some(_) => {
            record
                .origins
                .insert("display_name".to_owned(), SettingOrigin::Local);
        }
        None => {
            record.origins.remove("display_name");
        }
    }
    record.display_name = trimmed;
    Ok(())
}

/// T027 `SetFleetMemberLabels`: validate the whole map first, then write it.
///
/// `validate_labels` fails on the first offender in `BTreeMap` order and mutates nothing, so
/// validating before touching `file` is what makes "persist nothing on failure" true rather than
/// merely likely (FR-003).
///
/// Origins: a key is marked `Local` only when this edit *changed* it. A label adopted from a
/// shared definition and resubmitted unchanged stays shared, so FR-013's "a local value wins" does
/// not quietly claim every value the operator has ever seen. Keys dropped from the map lose their
/// origin with them.
fn set_labels(
    file: &mut FleetFile,
    context: &str,
    labels: BTreeMap<String, String>,
) -> Result<(), Rejection> {
    check_context(context)?;
    fleet_model::validate_labels(&labels).map_err(|e| Rejection {
        reason: e.to_string(),
        field: Some(e.part.noun().to_owned()),
    })?;

    let record = file.members.entry(context.to_owned()).or_default();
    let changed: Vec<&String> = labels
        .iter()
        .filter(|(key, value)| record.labels.get(*key) != Some(*value))
        .map(|(key, _)| key)
        .collect();
    let mut origins: BTreeMap<String, SettingOrigin> = record
        .origins
        .iter()
        .filter(|(key, _)| !key.starts_with("labels/"))
        .map(|(key, origin)| (key.clone(), *origin))
        .collect();
    for key in labels.keys() {
        let origin_key = format!("labels/{key}");
        let origin = if changed.contains(&key) {
            SettingOrigin::Local
        } else {
            record
                .origins
                .get(&origin_key)
                .copied()
                .unwrap_or(SettingOrigin::Local)
        };
        origins.insert(origin_key, origin);
    }
    record.origins = origins;
    record.labels = labels;
    Ok(())
}

/// T027 `ForgetFleetMember`: the only path that removes a member's metadata (FR-009).
///
/// It does not consult availability. An `Absent` member is precisely the one an operator most
/// wants to forget — the decommissioned cluster still on the dashboard — and refusing to forget it
/// because its context is gone would leave that row unremovable. Forgetting a member that was
/// never recorded is a no-op, not an error: the end state the operator asked for already holds.
fn forget_member(file: &mut FleetFile, context: &str) -> Result<(), Rejection> {
    check_context(context)?;
    file.members.remove(context.trim());
    Ok(())
}

/// T031 `SaveFleetGroup`: parse first, then upsert by name.
///
/// The selector is stored **verbatim as typed**, not as the parser's `Display`: `Expression::In`
/// holds a `BTreeSet`, so round-tripping `env in (stage,prod)` through the parser would hand the
/// operator back `env in (prod,stage)` — a different string from the one they wrote, in a field
/// whose whole purpose is to be the expression they wrote. Parsing is the gate; it is not the
/// storage format.
fn save_group(file: &mut FleetFile, name: &str, selector: &str) -> Result<(), Rejection> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Rejection::new("a group name must not be empty", "name"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(Rejection::new(
            format!(
                "group name \"{name}\" is invalid: must be 1-{MAX_NAME_CHARS} characters, and it is {}",
                name.chars().count()
            ),
            "name",
        ));
    }
    fleet_model::parse_selector(selector).map_err(|e| Rejection {
        reason: e.to_string(),
        field: Some("selector".to_owned()),
    })?;
    file.groups.insert(
        name.to_owned(),
        FleetGroupRecord {
            selector: selector.to_owned(),
            origin: SettingOrigin::Local,
        },
    );
    Ok(())
}

/// T031 `DeleteFleetGroup`: drop the group; members are untouched, because membership was never
/// stored on them in the first place (FR-004).
fn delete_group(file: &mut FleetFile, name: &str) -> Result<(), Rejection> {
    file.groups.remove(name.trim());
    Ok(())
}

/// T032 `RequestFleetGroupRollup`: aggregate what the backend already holds, and say what it
/// could not.
///
/// This command issues no cluster request of any kind. It reads `fleet.json`, the kubeconfig
/// context list, and `state.fleet.summaries` — the cache `fleet::emit_summary` fills on the way
/// out of the 027 poller and 034's live tier — and nothing else. There is no client built here and
/// no task spawned (FR-015, Principle V).
///
/// Three outcomes, deliberately distinct in what the UI receives:
/// - **no such group** → `FleetModelRejected`, field `group`. Not an empty rollup, which would
///   read as "the group matches nothing".
/// - **a stored selector that no longer parses** → `FleetModelRejected`, field `selector`, with
///   the parser's own message. Also not an empty rollup: a group Fathom cannot evaluate is not a
///   group that matches nobody (FR-005).
/// - **a group that genuinely matches nothing** → a rollup with `members_total: 0`, which says so.
fn request_rollup(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, group: &str) {
    let FleetLoad { file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
        return;
    }
    let name = group.trim();
    let Some(record) = file.groups.get(name) else {
        emit(
            state,
            sender,
            SystemEvent::FleetModelRejected {
                reason: format!("there is no fleet group named \"{name}\""),
                field: Some("group".to_owned()),
            },
        );
        return;
    };
    let selector = match fleet_model::parse_selector(&record.selector) {
        Ok(selector) => selector,
        Err(e) => {
            emit(
                state,
                sender,
                SystemEvent::FleetModelRejected {
                    reason: e.to_string(),
                    field: Some("selector".to_owned()),
                },
            );
            return;
        }
    };

    let contexts = kubeconfig_contexts(state);
    let (members, _) = model_from(&file, &contexts);
    let matching = members
        .iter()
        .filter(|m| fleet_model::selector_matches(&selector, &m.labels));
    let rollup = match state.fleet.summaries.lock() {
        Ok(summaries) => rollup_for(name, matching, &summaries),
        // A poisoned cache is not a licence to report an uncovered fleet as a healthy one: every
        // member becomes `Pending`, which is what "we hold no summary for it" means.
        Err(_) => rollup_for(name, matching, &BTreeMap::new()),
    };
    emit(state, sender, SystemEvent::FleetGroupRollupUpdated(rollup));
}

/// Aggregate the summaries held for `members`, and name every member that is not in the sum.
///
/// Every member lands in exactly one of two places — the sums or `gaps` — which is what makes
/// both invariants true by construction rather than by inspection:
/// `members_covered <= members_total`, and `gaps.len() == members_total - members_covered`.
///
/// The defect this is written against is `ui/src/resources/nav.ts`'s rendering of a *denied* CRD
/// scan as "not installed". A rollup that summed the three clusters it could read and called it
/// the group's health would be the same untruth with bigger numbers, so there is no path through
/// this function that drops a member silently.
fn rollup_for<'a>(
    group: &str,
    members: impl IntoIterator<Item = &'a FleetMember>,
    summaries: &BTreeMap<String, FleetContextSummary>,
) -> FleetGroupRollup {
    let mut rollup = FleetGroupRollup {
        group: group.to_owned(),
        members_total: 0,
        members_covered: 0,
        gaps: Vec::new(),
        node_ready: 0,
        node_total: 0,
        unhealthy_pods: 0,
    };
    for member in members {
        rollup.members_total = rollup.members_total.saturating_add(1);
        match covered_counts(member, summaries) {
            Ok(counts) => {
                rollup.members_covered = rollup.members_covered.saturating_add(1);
                rollup.node_ready = rollup.node_ready.saturating_add(counts.node_ready);
                rollup.node_total = rollup.node_total.saturating_add(counts.node_total);
                rollup.unhealthy_pods = rollup.unhealthy_pods.saturating_add(counts.unhealthy_pods);
            }
            Err(reason) => rollup.gaps.push(FleetCoverageGap {
                context: member.context.clone(),
                reason,
            }),
        }
    }
    debug_assert!(rollup.members_covered <= rollup.members_total);
    debug_assert_eq!(
        rollup.gaps.len() as u32,
        rollup.members_total - rollup.members_covered
    );
    rollup
}

/// What one covered member contributes to the sums.
struct MemberCounts {
    node_ready: u32,
    node_total: u32,
    unhealthy_pods: u32,
}

/// One member's contribution, or the reason it has none. The four reasons map to four distinct
/// states of the world and are never folded together (FR-011):
///
/// - `Absent` — the context is not in the current kubeconfig. Nothing was asked of it, because
///   there is nothing to ask.
/// - `Pending` — in the kubeconfig, but no summary has arrived yet (no poll, or the poller's own
///   `Pending` placeholder). It was asked and has not answered.
/// - `NotPermitted` — a 403. Either the credentials were refused outright
///   (`FleetHealthState::AuthError`, whose `error` is the API server's own message verbatim) or
///   the cluster answered but denied a resource group, which 027 records in `restricted` with the
///   matching counts `None`. A member whose node or pod list was forbidden is **not** covered:
///   counting it would state a sum over numbers nobody was allowed to read.
/// - `Unreachable` — the connection or the API call failed, with the transport error verbatim.
fn covered_counts(
    member: &FleetMember,
    summaries: &BTreeMap<String, FleetContextSummary>,
) -> Result<MemberCounts, FleetGapReason> {
    if member.availability == FleetMemberAvailability::Absent {
        return Err(FleetGapReason::Absent);
    }
    let Some(summary) = summaries.get(&member.context) else {
        return Err(FleetGapReason::Pending);
    };
    match summary.health {
        FleetHealthState::Pending => Err(FleetGapReason::Pending),
        FleetHealthState::AuthError => Err(FleetGapReason::NotPermitted {
            detail: reported_detail(summary),
        }),
        FleetHealthState::Unreachable => Err(FleetGapReason::Unreachable {
            detail: reported_detail(summary),
        }),
        FleetHealthState::Healthy | FleetHealthState::Degraded => {
            if !summary.restricted.is_empty() {
                return Err(FleetGapReason::NotPermitted {
                    detail: restricted_detail(summary),
                });
            }
            match (
                summary.node_ready_count,
                summary.node_total_count,
                summary.unhealthy_pod_count,
            ) {
                (Some(node_ready), Some(node_total), Some(unhealthy_pods)) => Ok(MemberCounts {
                    node_ready,
                    node_total,
                    unhealthy_pods,
                }),
                // Reachable, nothing denied, and still no counts: the summary does not say this
                // member was read, so it is not covered. `Pending` is the honest reading.
                _ => Err(FleetGapReason::Pending),
            }
        }
    }
}

/// The cluster's own words for a failure, never Fathom's (Principle I). 027 puts the API server's
/// or transport's message in `error` verbatim; `plugin_error` carries 035's typed credential
/// failure when the exec plugin is what broke.
fn reported_detail(summary: &FleetContextSummary) -> String {
    if let Some(error) = &summary.error {
        return error.clone();
    }
    if let Some(plugin) = &summary.plugin_error {
        return plugin.message.clone();
    }
    "the cluster reported no reason".to_owned()
}

/// A partial denial's detail. 027's `FleetContextSummary` keeps only the denied resource groups,
/// not the API server's 403 body, so this names the groups it was refused — the most specific
/// truth the summary actually holds. (Carrying the server's message through
/// `fleet::restrict_forbidden` would change 027's summary shape; noted for a follow-up.)
fn restricted_detail(summary: &FleetContextSummary) -> String {
    format!(
        "the API server refused to list {} for these credentials",
        summary.restricted.join(" and ")
    )
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
    let FleetLoad { file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
    }
    emit_model(state, sender, &file);
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

    let mut members: Vec<FleetMember> = names
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
                // Filled in by `resolve_membership` below, once the selectors are parsed.
                groups: Vec::new(),
            }
        })
        .collect();

    let mut groups: Vec<FleetGroup> = file
        .groups
        .iter()
        .map(|(name, record)| FleetGroup {
            name: name.clone(),
            selector: record.selector.clone(),
            origin: record.origin,
            selector_error: None,
        })
        .collect();

    resolve_membership(&mut members, &mut groups);
    (members, groups)
}

/// Resolve group membership once, here, and record what each group reported (FR-004).
///
/// This is the **only** evaluation of the selector grammar: `src/backend/fleet_model.rs` parses
/// the expression and `FleetMember::groups` carries the answer to the UI, so nothing downstream
/// has to re-derive it (Principle II). A group whose *stored* selector no longer parses — a file
/// edited outside Fathom, or written by an older version — contributes to no member's list and
/// records the parser's verbatim message in `selector_error`. That is what keeps it
/// distinguishable from a group that parses and matches nothing (FR-005): the latter has an empty
/// membership and `selector_error: None`, and the UI reads the difference without parsing a thing.
fn resolve_membership(members: &mut [FleetMember], groups: &mut [FleetGroup]) {
    let mut parsed: Vec<(String, Selector)> = Vec::with_capacity(groups.len());
    for group in groups.iter_mut() {
        match fleet_model::parse_selector(&group.selector) {
            Ok(selector) => {
                group.selector_error = None;
                parsed.push((group.name.clone(), selector));
            }
            Err(e) => {
                // Verbatim, like every other refusal this feature reports (Principle I).
                group.selector_error = Some(e.to_string());
                tracing::warn!(
                    "fleet group {:?} has an unparseable selector and matches nothing: {}",
                    group.name,
                    e
                );
            }
        }
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for member in members.iter_mut() {
        member.groups = fleet_model::groups_matching(
            parsed
                .iter()
                .map(|(name, selector)| (name.as_str(), selector)),
            &member.labels,
        )
        .into_iter()
        .map(str::to_owned)
        .collect();
        for name in &member.groups {
            *counts.entry(name.clone()).or_default() += 1;
        }
    }
    tracing::debug!(
        "fleet model: {} members, {} groups, membership {:?}",
        members.len(),
        groups.len(),
        counts
    );
}

// ---------------------------------------------------------------------------------------
// T040-T042 (US3): one definition of "prod", shared through the 014 store.
//
// Everything in this section obeys one rule above all the others: **nothing here deletes an
// adopted copy** (FR-014, Principle VI). A store that cannot be reached, a definition
// withdrawn upstream, a definition that vanished from the store entirely — all three flip
// `AdoptedFleetDefinition::still_shared` to `false` and do nothing else. There is no code
// path in this file that removes a key from `FleetFile::adopted`.
// ---------------------------------------------------------------------------------------

/// The 014 store root, or `StoreError`'s own words for why there is none.
///
/// All shared-store I/O for the fleet model happens here, in the backend; the UI never
/// touches the store (Principle II). The path is the one 014 already configures — this
/// feature adds no second place to point an install at (FR-012).
fn store_root() -> Result<PathBuf, StoreError> {
    persistence::load_team_config()
        .store_path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| StoreError::Unreachable("no shared config store configured".to_owned()))
}

/// 033's role map gates publishing, exactly as it gates `PublishConnectionProfile`
/// (`src/backend/cmd/team.rs`): the same `require_role` against the same persisted map, with
/// the same rule that a kubeconfig-only context (no OIDC identity in play) is unrestricted.
///
/// It is enforced **here**, in the handler, and deliberately not in
/// `src/protocol/v1/mutation.rs`: the hosted policy table decides what a *deployment* may
/// ask for, and a role is a property of who is asking, which that table cannot see. A
/// refusal is reported, never silent — `FleetModelRejected` with the directory's own reason
/// and `field: None`, because no value the operator typed is at fault.
fn gate(state: &BackendState, sender: &Sender<BackendEventEnvelope>, required: FathomRole) -> bool {
    match super::role_check(state, required) {
        Ok(()) => true,
        Err(reason) => {
            emit(
                state,
                sender,
                SystemEvent::FleetModelRejected {
                    reason,
                    field: None,
                },
            );
            false
        }
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Attribution, recorded the way 014 records it (`team.rs::current_operator`).
fn current_operator() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "operator".to_owned())
}

/// Emit the store's rows and **every** adopted row, including the ones that are no longer
/// following (FR-013, FR-014).
///
/// Neither list is filtered. A `removed` store row is still emitted, because an operator who
/// adopted it has to be able to see that it was withdrawn; and an adopted row with
/// `still_shared: false` is still emitted, because dropping it from the event is exactly how
/// an operator's adopted copy would appear to vanish from the UI while sitting untouched in
/// `fleet.json` — indistinguishable, to them, from the deletion FR-014 forbids.
fn emit_definitions(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    definitions: Vec<SharedFleetDefinition>,
    file: &FleetFile,
) {
    emit(
        state,
        sender,
        SystemEvent::SharedFleetDefinitionsUpdated {
            definitions,
            adopted: file.adopted.values().cloned().collect(),
        },
    );
}

/// The store's rows, or none when it cannot be read. Used only on the way out of a command
/// that already reported any failure of its own.
fn definitions_in(root: &Path) -> Vec<SharedFleetDefinition> {
    team_store::read_fleet_definitions(root).unwrap_or_default()
}

/// Mark every adopted definition as no longer following the shared copy.
///
/// This is the *entire* local effect of losing the store. It flips one `bool` per row and
/// removes nothing; `file.adopted`'s key set is identical before and after. Returns whether
/// anything changed, so a store failure with nothing adopted writes no file at all.
fn stop_following(file: &mut FleetFile) -> bool {
    let mut changed = false;
    for adopted in file.adopted.values_mut() {
        if adopted.still_shared {
            adopted.still_shared = false;
            changed = true;
        }
    }
    changed
}

/// Report a store failure in `StoreError`'s own words (FR-012) and stop following, without
/// deleting anything (FR-014).
///
/// A `Conflict` is not a lost store — the store answered, and loudly — so it reports the
/// conflict and leaves `still_shared` alone. An `Unreachable` or `Invalid` store is one
/// nothing can be following, so every adopted row records that, and keeps everything else.
///
/// The file is re-loaded here rather than passed in so that every command's failure path is
/// one call. A `fleet.json` that does not parse is reported and **not written over**
/// (Principle VI): a store failure is no reason to turn an unreadable file into a lost one.
fn report_store_failure(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    error: &StoreError,
) {
    emit(
        state,
        sender,
        SystemEvent::FleetStoreUnavailable {
            reason: error.to_string(),
        },
    );
    let FleetLoad { mut file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
        return;
    }
    if !matches!(error, StoreError::Conflict { .. })
        && stop_following(&mut file)
        && !save_or_report(state, sender, &file)
    {
        return;
    }
    emit_definitions(state, sender, Vec::new(), &file);
}

/// Persist the model, reporting a disk refusal the way `edit_model` does: the edit was valid
/// but nothing was written, which is what `FleetModelRejected` promises. `field` is `None` —
/// no input is at fault.
fn save_or_report(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    file: &FleetFile,
) -> bool {
    match persistence::save_fleet_model(file) {
        Ok(()) => true,
        Err(e) => {
            emit(
                state,
                sender,
                SystemEvent::FleetModelRejected {
                    reason: format!("{e:#}"),
                    field: None,
                },
            );
            false
        }
    }
}

/// T040 `RefreshSharedFleetDefinitions`: read the store, reconcile what was adopted, emit.
///
/// Reads two files (`fleet.json`, `fleet-definitions.json`) and opens no connection
/// (FR-015). An unreachable store is not an error that ends the command: the adopted copies
/// are reported exactly as they stand, marked as no longer following (FR-014).
fn refresh_shared_definitions(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    let FleetLoad { mut file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
        return;
    }
    let shared = match store_root().and_then(|root| team_store::read_fleet_definitions(&root)) {
        Ok(shared) => shared,
        Err(e) => {
            report_store_failure(state, sender, &e);
            return;
        }
    };
    if reconcile_adopted(&mut file, &shared) && !save_or_report(state, sender, &file) {
        return;
    }
    emit_definitions(state, sender, shared, &file);
}

/// Bring every adopted copy up to date with the store, and *mark* — never drop — the ones
/// that are no longer shared (FR-014). Returns whether the file changed.
///
/// Two of the three ways to stop following are here: the upstream row is `removed: true`, or
/// it is gone from the store entirely. (The third, an unreachable store, is
/// `report_store_failure`.) Neither branch touches `file.adopted`'s keys, and the `removed`
/// branch keeps the definition as adopted, so the shared values stay readable.
fn reconcile_adopted(file: &mut FleetFile, shared: &[SharedFleetDefinition]) -> bool {
    let mut changed = false;
    for (id, adopted) in file.adopted.iter_mut() {
        match shared.iter().find(|d| &d.id == id) {
            Some(d) if !d.removed => {
                if adopted.definition != *d || !adopted.still_shared {
                    adopted.definition = d.clone();
                    adopted.still_shared = true;
                    changed = true;
                }
            }
            // Removed upstream, or no longer in the store at all. The row stays; only the
            // flag moves.
            _ => {
                if adopted.still_shared {
                    adopted.still_shared = false;
                    changed = true;
                }
            }
        }
    }
    changed
}

/// The `rev` this operator last saw for `id`: their own adopted copy's, falling back to the
/// row currently in the store, and `0` for a definition nobody has published yet.
///
/// Preferring the adopted copy is what makes the concurrency check mean something: an
/// operator whose local copy is stale gets `StoreError::Conflict` instead of overwriting a
/// teammate's newer definition.
fn last_seen_rev(file: &FleetFile, root: &Path, id: &str) -> u64 {
    if let Some(adopted) = file.adopted.get(id) {
        return adopted.definition.rev;
    }
    definitions_in(root)
        .into_iter()
        .find(|d| d.id == id)
        .map_or(0, |d| d.rev)
}

/// T040 `PublishFleetDefinition`: the local labels and groups, as a shared definition.
///
/// Gated on `FathomRole::Editor` by the caller. Publishing also records the publisher's own
/// adoption, as `publish_connection_profile` does, so their copy follows what they published
/// and a later `rev` from a teammate is reconciled rather than fought over.
///
/// Only `display_name` and `labels` travel: `FleetMemberShare` has no field capable of
/// carrying a credential (FR-020), and group membership is not published at all because it
/// is computed from labels, never stored (FR-004).
fn publish_definition(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, id: &str) {
    let FleetLoad { mut file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
        return;
    }
    let id = id.trim().to_owned();
    if id.is_empty() {
        emit(
            state,
            sender,
            SystemEvent::FleetModelRejected {
                reason: "a shared fleet definition needs an id".to_owned(),
                field: Some("id".to_owned()),
            },
        );
        return;
    }
    let root = match store_root() {
        Ok(root) => root,
        Err(e) => {
            report_store_failure(state, sender, &e);
            return;
        }
    };
    let expected_rev = last_seen_rev(&file, &root, &id);
    let definition = SharedFleetDefinition {
        id: id.clone(),
        members: file
            .members
            .iter()
            .map(|(context, record)| {
                (
                    context.clone(),
                    FleetMemberShare {
                        display_name: record.display_name.clone(),
                        labels: record.labels.clone(),
                    },
                )
            })
            .collect(),
        groups: file
            .groups
            .iter()
            .map(|(name, record)| (name.clone(), record.selector.clone()))
            .collect(),
        rev: expected_rev,
        shared_by: current_operator(),
        updated_at: now_rfc3339(),
        removed: false,
    };
    let rev = match team_store::publish_fleet_definition(&root, definition.clone(), expected_rev) {
        Ok(rev) => rev,
        Err(e) => {
            report_store_failure(state, sender, &e);
            return;
        }
    };
    file.adopted.insert(
        id,
        AdoptedFleetDefinition {
            definition: SharedFleetDefinition { rev, ..definition },
            adopted_at: now_rfc3339(),
            still_shared: true,
        },
    );
    if !save_or_report(state, sender, &file) {
        return;
    }
    emit_model(state, sender, &file);
    emit_definitions(state, sender, definitions_in(&root), &file);
}

/// T040 `AdoptFleetDefinition`: copy a shared definition locally, overwriting nothing local.
///
/// The store is read first and the model is written through `edit_model`, so adoption
/// inherits that path's two promises: nothing is persisted on a refusal, and an unparseable
/// `fleet.json` is reported rather than overwritten (Principle VI).
fn adopt_definition(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, id: &str) {
    let id = id.trim().to_owned();
    let root = match store_root() {
        Ok(root) => root,
        Err(e) => {
            report_store_failure(state, sender, &e);
            return;
        }
    };
    let shared = match team_store::read_fleet_definitions(&root) {
        Ok(shared) => shared,
        Err(e) => {
            report_store_failure(state, sender, &e);
            return;
        }
    };
    let Some(definition) = shared.into_iter().find(|d| d.id == id && !d.removed) else {
        // Not a store failure and not an empty adoption: there is nothing by that name to
        // adopt, and saying so names the id the operator asked for.
        emit(
            state,
            sender,
            SystemEvent::FleetModelRejected {
                reason: format!("there is no shared fleet definition \"{id}\" to adopt"),
                field: Some("id".to_owned()),
            },
        );
        return;
    };
    if !edit_model(state, sender, |file| adopt_into(file, definition)) {
        return;
    }
    let FleetLoad { file, failure } = persistence::load_fleet_model();
    if failure.is_none() {
        emit_definitions(state, sender, definitions_in(&root), &file);
    }
}

/// FR-013: adoption fills in what the operator has not decided for themselves, and
/// overwrites nothing they have.
///
/// - **A `Local` value wins and stays `Local`.** Not "wins this time": the origin is left
///   untouched, so the next adoption leaves it alone too.
/// - **The shared value it overrode stays readable.** It lives in `file.adopted[id]
///   .definition`, which keeps the whole `SharedFleetDefinition` verbatim — the shared
///   display name and every shared label, including the ones a local value is currently
///   winning over. Nothing flattens the shared side into `FleetMemberRecord`, so there is no
///   step at which the overridden value could be lost, and
///   `SharedFleetDefinitionsUpdated` hands the UI both sides.
/// - **Everything adoption writes is marked `OrgDefault`.** There is no `Shared` origin;
///   `OrgDefault` *is* the shared value (`SettingOrigin`).
///
/// Labels are validated before anything is written, so a shared definition with an invalid
/// label refuses the whole adoption rather than persisting half of it. Selectors are stored
/// verbatim and not parsed here: an unparseable shared selector surfaces as the group's
/// `selector_error` through `resolve_membership`, exactly as a hand-edited file does
/// (FR-005), which keeps "cannot be evaluated" distinct from "matches nothing".
fn adopt_into(file: &mut FleetFile, definition: SharedFleetDefinition) -> Result<(), Rejection> {
    for (context, share) in &definition.members {
        check_context(context)?;
        fleet_model::validate_labels(&share.labels).map_err(|e| Rejection {
            reason: e.to_string(),
            field: Some(e.part.noun().to_owned()),
        })?;
    }
    for (context, share) in &definition.members {
        let record = file.members.entry(context.clone()).or_default();
        adopt_display_name(record, share);
        adopt_labels(record, share);
    }
    for (name, selector) in &definition.groups {
        // A locally-defined group wins exactly as a local label does.
        if file.groups.get(name).map(|g| g.origin) == Some(SettingOrigin::Local) {
            continue;
        }
        file.groups.insert(
            name.clone(),
            FleetGroupRecord {
                selector: selector.clone(),
                origin: SettingOrigin::OrgDefault,
            },
        );
    }
    file.adopted.insert(
        definition.id.clone(),
        AdoptedFleetDefinition {
            definition,
            adopted_at: now_rfc3339(),
            still_shared: true,
        },
    );
    Ok(())
}

/// The shared display name, unless the operator set their own. A share that names nothing
/// clears nothing: the absence of a shared name is not an instruction to discard a name the
/// operator already has (Principle VI).
fn adopt_display_name(record: &mut FleetMemberRecord, share: &FleetMemberShare) {
    if record.origins.get("display_name") == Some(&SettingOrigin::Local) {
        return;
    }
    if let Some(name) = &share.display_name {
        record.display_name = Some(name.clone());
        record
            .origins
            .insert("display_name".to_owned(), SettingOrigin::OrgDefault);
    }
}

/// The shared labels, key by key, skipping every key the operator set locally. Keys the
/// share does not mention are left alone — adoption adds and updates, it never prunes.
fn adopt_labels(record: &mut FleetMemberRecord, share: &FleetMemberShare) {
    for (key, value) in &share.labels {
        let origin_key = format!("labels/{key}");
        if record.origins.get(&origin_key) == Some(&SettingOrigin::Local) {
            continue;
        }
        record.labels.insert(key.clone(), value.clone());
        record.origins.insert(origin_key, SettingOrigin::OrgDefault);
    }
}

/// T040 `RemoveFleetDefinition`: withdraw the store row, keep every adopted copy.
///
/// The store row is marked `removed` rather than erased (`team_store::remove_fleet_definition`),
/// and this operator's own adopted copy stays in `fleet.json` with `still_shared: false`.
/// Un-publishing is not a delete, here or upstream (FR-014).
fn remove_definition(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, id: &str) {
    let FleetLoad { mut file, failure } = persistence::load_fleet_model();
    if let Some(reason) = failure {
        emit(
            state,
            sender,
            SystemEvent::FleetModelLoadFailed {
                path: fleet_path_display(),
                reason,
            },
        );
        return;
    }
    let id = id.trim().to_owned();
    let root = match store_root() {
        Ok(root) => root,
        Err(e) => {
            report_store_failure(state, sender, &e);
            return;
        }
    };
    let expected_rev = last_seen_rev(&file, &root, &id);
    if let Err(e) = team_store::remove_fleet_definition(&root, &id, expected_rev) {
        report_store_failure(state, sender, &e);
        return;
    }
    if let Some(adopted) = file.adopted.get_mut(&id) {
        adopted.still_shared = false;
    }
    if !save_or_report(state, sender, &file) {
        return;
    }
    emit_model(state, sender, &file);
    emit_definitions(state, sender, definitions_in(&root), &file);
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
        // The answer travels in the event: no caller re-evaluates the selector (Principle II).
        let matched: Vec<&str> = members
            .iter()
            .filter(|m| m.groups.iter().any(|g| g == &groups[0].name))
            .map(|m| m.context.as_str())
            .collect();
        assert_eq!(matched, ["prod-a", "retired"]);
        // A group that parses carries no error, whether or not it matched anyone.
        assert!(groups.iter().all(|g| g.selector_error.is_none()));
    }

    /// A member matches every group it matches, and the list is the group definition order.
    #[test]
    fn a_member_in_two_groups_names_both() {
        let mut f = file();
        f.groups.insert(
            "all".to_owned(),
            FleetGroupRecord {
                selector: "env".to_owned(),
                origin: SettingOrigin::Local,
            },
        );
        let (members, _) = model_from(&f, &["prod-a".to_owned()]);
        let prod_a = members.iter().find(|m| m.context == "prod-a").unwrap();
        assert_eq!(prod_a.groups, ["all".to_owned(), "prod".to_owned()]);
    }

    /// FR-005/FR-011: "matches nothing" and "cannot be read" are two states, not one. A group
    /// that parses and matches nobody has an empty membership and no error; a group whose stored
    /// selector no longer parses has an empty membership *and* the parser's verbatim message, and
    /// appears in no member's `groups`.
    #[test]
    fn a_group_that_matches_nothing_reads_differently_from_one_that_cannot_be_parsed() {
        let mut f = file();
        f.groups.insert(
            "empty".to_owned(),
            FleetGroupRecord {
                selector: "env=nowhere".to_owned(),
                origin: SettingOrigin::Local,
            },
        );
        f.groups.insert(
            "broken".to_owned(),
            FleetGroupRecord {
                selector: "env = = prod".to_owned(),
                origin: SettingOrigin::Local,
            },
        );
        let (members, groups) = model_from(&f, &["prod-a".to_owned()]);

        let find = |name: &str| {
            groups
                .iter()
                .find(|g| g.name == name)
                .unwrap_or_else(|| panic!("no group {name}"))
        };
        assert_eq!(find("empty").selector_error, None);
        let expected = fleet_model::parse_selector("env = = prod")
            .unwrap_err()
            .to_string();
        assert_eq!(find("broken").selector_error.as_deref(), Some(&*expected));

        // Neither holds a member — and the unparseable one is in nobody's list at all.
        for member in &members {
            assert!(!member.groups.iter().any(|g| g == "empty"));
            assert!(
                !member.groups.iter().any(|g| g == "broken"),
                "an unparseable selector must not resolve any membership"
            );
        }
    }

    /// The four 014-sharing commands are US3's (T039-T042) and must still fall through to the
    /// next family, so they behave exactly as they did before this family existed rather than
    /// being silently swallowed here.
    #[test]
    fn unclaimed_commands_are_handed_back() {
        // T040 claimed all four sharing commands, so they are handled here now. A temp
        // state root keeps the dispatch off the operator's real `~/.fathom`.
        let dir = tempfile::tempdir().unwrap();
        crate::backend::persistence::set_thread_state_root(dir.path().to_path_buf());
        let mut state = BackendState::new();
        let (tx, _rx) = crossbeam_channel::unbounded();
        for cmd in [
            BackendCmd::RefreshSharedFleetDefinitions,
            BackendCmd::PublishFleetDefinition {
                id: "prod".to_owned(),
            },
            BackendCmd::AdoptFleetDefinition {
                id: "prod".to_owned(),
            },
            BackendCmd::RemoveFleetDefinition {
                id: "prod".to_owned(),
            },
        ] {
            let back = handle(cmd, &mut state, &tx).unwrap();
            assert!(back.is_none(), "a US3 command is handled by this family");
        }
        // A command from another family is still handed straight back.
        let back = handle(BackendCmd::RefreshTeamStoreStatus, &mut state, &tx).unwrap();
        assert!(
            matches!(back, Some(BackendCmd::RefreshTeamStoreStatus)),
            "another family's command must fall through untouched"
        );
    }

    // -----------------------------------------------------------------------------------
    // T026 / T027 / T031: the editing commands.
    // -----------------------------------------------------------------------------------

    /// Run `cmd` against a fleet file in a throwaway state root, and return the file's bytes
    /// afterwards together with every system event emitted. `set_thread_state_root` is
    /// thread-local and every test gets its own thread, so this does not leak between tests.
    fn run(initial: &FleetFile, cmd: BackendCmd) -> (Vec<u8>, Vec<SystemEvent>) {
        let dir = tempfile::tempdir().unwrap();
        crate::backend::persistence::set_thread_state_root(dir.path().to_path_buf());
        let path = crate::backend::persistence::fleet_file_path().unwrap();
        crate::backend::persistence::save_fleet_model(initial).unwrap();

        let mut state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();
        handle(cmd, &mut state, &tx).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let events = rx
            .try_iter()
            .filter_map(|env| match env.event {
                BackendEvent::System(event) => Some(event),
                _ => None,
            })
            .collect();
        (bytes, events)
    }

    fn saved(bytes: &[u8]) -> FleetFile {
        serde_json::from_slice(bytes).unwrap()
    }

    /// T026: trimmed, and empty-after-trim is `None` — never `Some("")`, which would render as
    /// a blank where FR-002 promises a name beside the context.
    #[test]
    fn a_display_name_is_trimmed_and_an_empty_one_becomes_absent() {
        let (bytes, events) = run(
            &file(),
            BackendCmd::SetFleetMemberDisplayName {
                context: "prod-a".to_owned(),
                display_name: Some("  london prod  ".to_owned()),
            },
        );
        let record = &saved(&bytes).members["prod-a"];
        assert_eq!(record.display_name.as_deref(), Some("london prod"));
        assert_eq!(record.origins["display_name"], SettingOrigin::Local);
        assert!(matches!(
            events.last(),
            Some(SystemEvent::FleetModelUpdated { .. })
        ));

        for cleared in [Some("   ".to_owned()), None] {
            let (bytes, _) = run(
                &file(),
                BackendCmd::SetFleetMemberDisplayName {
                    context: "prod-a".to_owned(),
                    display_name: cleared.clone(),
                },
            );
            let record = &saved(&bytes).members["prod-a"];
            assert_eq!(record.display_name, None, "cleared by {cleared:?}");
            assert!(
                !record.origins.contains_key("display_name"),
                "a cleared name has no local origin to report"
            );
        }
    }

    /// T026: 129 characters is refused by name, and the file is not touched.
    #[test]
    fn an_over_long_display_name_is_refused() {
        let before = file();
        let (bytes, events) = run(
            &before,
            BackendCmd::SetFleetMemberDisplayName {
                context: "prod-a".to_owned(),
                display_name: Some("x".repeat(MAX_NAME_CHARS + 1)),
            },
        );
        assert_eq!(saved(&bytes), before, "nothing may be persisted");
        let Some(SystemEvent::FleetModelRejected { reason, field }) = events.last() else {
            panic!("expected a rejection, got {events:?}");
        };
        assert_eq!(field.as_deref(), Some("display_name"));
        assert!(
            reason.contains("129"),
            "the reason names the length: {reason}"
        );
    }

    /// T027 (FR-003): the validator's message reaches the UI verbatim, the field it names is
    /// the validator's own noun, and `fleet.json` is byte-for-byte what it was.
    #[test]
    fn an_invalid_label_is_refused_verbatim_and_persists_nothing() {
        let before = file();
        let bad: BTreeMap<String, String> = BTreeMap::from([
            ("env".to_owned(), "prod".to_owned()),
            ("tier/of/too/many".to_owned(), "gold".to_owned()),
        ]);
        let expected = fleet_model::validate_labels(&bad).unwrap_err();
        let (bytes, events) = run(
            &before,
            BackendCmd::SetFleetMemberLabels {
                context: "prod-a".to_owned(),
                labels: bad,
            },
        );
        assert_eq!(saved(&bytes), before, "a refused edit persists nothing");
        let Some(SystemEvent::FleetModelRejected { reason, field }) = events.last() else {
            panic!("expected a rejection, got {events:?}");
        };
        assert_eq!(reason, &expected.to_string(), "the message is not reworded");
        assert_eq!(field.as_deref(), Some(expected.part.noun()));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SystemEvent::FleetModelUpdated { .. })),
            "a rejection is emitted instead of an update, never alongside it"
        );
    }

    /// T027: the whole map is validated before anything is written, so the *valid* entries of a
    /// map with one offender do not land either.
    #[test]
    fn a_map_with_one_offender_writes_none_of_its_valid_entries() {
        let mut file = file();
        let err = set_labels(
            &mut file,
            "prod-a",
            BTreeMap::from([
                ("aaa".to_owned(), "ok".to_owned()),
                ("zzz".to_owned(), "NOT A VALUE".to_owned()),
            ]),
        )
        .unwrap_err();
        assert_eq!(err.field.as_deref(), Some("label value"));
        assert_eq!(
            file.members["prod-a"].labels,
            BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
            "the record is untouched, including the key that was valid"
        );
    }

    /// T027 (FR-013): only the keys this edit changed become local. A shared
    /// (`SettingOrigin::OrgDefault`) label resubmitted unchanged stays shared, so adoption
    /// (T040) can still update it.
    #[test]
    fn only_a_changed_label_becomes_local() {
        let mut file = FleetFile {
            members: BTreeMap::from([(
                "prod-a".to_owned(),
                FleetMemberRecord {
                    display_name: None,
                    labels: BTreeMap::from([
                        ("env".to_owned(), "prod".to_owned()),
                        ("tier".to_owned(), "gold".to_owned()),
                    ]),
                    origins: BTreeMap::from([
                        ("labels/env".to_owned(), SettingOrigin::OrgDefault),
                        ("labels/tier".to_owned(), SettingOrigin::OrgDefault),
                    ]),
                },
            )]),
            ..FleetFile::default()
        };
        set_labels(
            &mut file,
            "prod-a",
            BTreeMap::from([
                ("env".to_owned(), "prod".to_owned()),
                ("tier".to_owned(), "silver".to_owned()),
            ]),
        )
        .unwrap();
        let origins = &file.members["prod-a"].origins;
        assert_eq!(origins["labels/env"], SettingOrigin::OrgDefault);
        assert_eq!(origins["labels/tier"], SettingOrigin::Local);
    }

    /// T027 (FR-009): forgetting works whether the member is `Present` or `Absent`. `retired` is
    /// absent from the kubeconfig here and is still removable — it is the row an operator most
    /// wants gone.
    #[test]
    fn forgetting_removes_metadata_for_a_present_and_an_absent_member() {
        for context in ["prod-a", "retired"] {
            let (bytes, events) = run(
                &file(),
                BackendCmd::ForgetFleetMember {
                    context: context.to_owned(),
                },
            );
            assert!(
                !saved(&bytes).members.contains_key(context),
                "{context} should be forgotten"
            );
            assert!(matches!(
                events.last(),
                Some(SystemEvent::FleetModelUpdated { .. })
            ));
        }
    }

    /// T031: the selector is stored exactly as typed. `Expression::In` holds a `BTreeSet`, so
    /// the parser's own `Display` would hand back `env in (prod,stage)` — a string the operator
    /// never wrote.
    #[test]
    fn a_selector_is_stored_verbatim_not_normalized() {
        let (bytes, _) = run(
            &FleetFile::default(),
            BackendCmd::SaveFleetGroup {
                name: "  staging-and-prod ".to_owned(),
                selector: "env in (stage,prod)".to_owned(),
            },
        );
        let saved = saved(&bytes);
        let group = &saved.groups["staging-and-prod"];
        assert_eq!(group.selector, "env in (stage,prod)");
        assert_eq!(group.origin, SettingOrigin::Local);
    }

    /// T031 (FR-006): a parse error is reported verbatim against `selector`, and no group is
    /// created — not an empty one, not a disabled one.
    #[test]
    fn an_unparseable_selector_creates_no_group() {
        let expected = fleet_model::parse_selector("env in prod").unwrap_err();
        let (bytes, events) = run(
            &FleetFile::default(),
            BackendCmd::SaveFleetGroup {
                name: "prod".to_owned(),
                selector: "env in prod".to_owned(),
            },
        );
        assert!(saved(&bytes).groups.is_empty(), "no group may be created");
        let Some(SystemEvent::FleetModelRejected { reason, field }) = events.last() else {
            panic!("expected a rejection, got {events:?}");
        };
        assert_eq!(reason, &expected.to_string());
        assert!(
            reason.contains(&expected.offending),
            "the offending substring survives"
        );
        assert_eq!(field.as_deref(), Some("selector"));
    }

    /// T031: upsert by name, and deleting a group leaves every member's labels alone.
    #[test]
    fn saving_upserts_by_name_and_deleting_leaves_members_alone() {
        let mut file = file();
        save_group(&mut file, "prod", "env=prod").unwrap();
        assert_eq!(file.groups.len(), 1, "upsert, not a second group");
        assert_eq!(file.groups["prod"].selector, "env=prod");
        delete_group(&mut file, "prod").unwrap();
        assert!(file.groups.is_empty());
        assert_eq!(file.members["prod-a"].labels["env"], "prod");
    }

    // -----------------------------------------------------------------------------------
    // T032 / T033: the rollup and its coverage.
    // -----------------------------------------------------------------------------------

    fn summary(context: &str, health: FleetHealthState) -> FleetContextSummary {
        FleetContextSummary {
            context: context.to_owned(),
            health,
            error: None,
            node_ready_count: None,
            node_total_count: None,
            unhealthy_pod_count: None,
            cluster_version: None,
            flux: None,
            restricted: Vec::new(),
            last_updated: "2026-01-01T00:00:00Z".to_owned(),
            live: false,
            plugin_error: None,
        }
    }

    fn member(context: &str, availability: FleetMemberAvailability) -> FleetMember {
        FleetMember {
            context: context.to_owned(),
            display_name: None,
            labels: BTreeMap::from([("env".to_owned(), "prod".to_owned())]),
            origins: BTreeMap::new(),
            availability,
            groups: Vec::new(),
        }
    }

    /// T033 (FR-010, FR-011): a mixed fleet — one covered, one denied outright, one denied a
    /// resource group, one unreachable, one absent, one not yet polled.
    ///
    /// Both invariants are asserted, and so is the *distinctness* of the reasons: a test that
    /// only counted gaps would pass against a version that collapsed all four into one, which is
    /// precisely the `integrationVisible` defect this feature exists to stop repeating.
    #[test]
    fn a_mixed_fleet_rollup_states_its_own_coverage() {
        let members = vec![
            member("covered", FleetMemberAvailability::Present),
            member("denied", FleetMemberAvailability::Present),
            member("partly-denied", FleetMemberAvailability::Present),
            member("unreachable", FleetMemberAvailability::Present),
            member("retired", FleetMemberAvailability::Absent),
            member("unpolled", FleetMemberAvailability::Present),
        ];
        let summaries = BTreeMap::from([
            (
                "covered".to_owned(),
                FleetContextSummary {
                    node_ready_count: Some(2),
                    node_total_count: Some(3),
                    unhealthy_pod_count: Some(4),
                    ..summary("covered", FleetHealthState::Degraded)
                },
            ),
            (
                "denied".to_owned(),
                FleetContextSummary {
                    error: Some(
                        "nodes is forbidden: User \"alice\" cannot list resource \"nodes\" in API group \"\" at the cluster scope"
                            .to_owned(),
                    ),
                    ..summary("denied", FleetHealthState::AuthError)
                },
            ),
            (
                "partly-denied".to_owned(),
                FleetContextSummary {
                    restricted: vec!["nodes".to_owned()],
                    unhealthy_pod_count: Some(9),
                    ..summary("partly-denied", FleetHealthState::Healthy)
                },
            ),
            (
                "unreachable".to_owned(),
                FleetContextSummary {
                    error: Some("error trying to connect: tcp connect error: timed out".to_owned()),
                    ..summary("unreachable", FleetHealthState::Unreachable)
                },
            ),
            // `retired` has a stale summary on purpose: absence from the kubeconfig wins, so a
            // forgotten cluster's last numbers can never be counted as current coverage.
            (
                "retired".to_owned(),
                FleetContextSummary {
                    node_ready_count: Some(100),
                    node_total_count: Some(100),
                    unhealthy_pod_count: Some(0),
                    ..summary("retired", FleetHealthState::Healthy)
                },
            ),
        ]);

        let rollup = rollup_for("prod", &members, &summaries);

        // Invariant 1 and 2.
        assert_eq!(rollup.members_total, 6);
        assert_eq!(rollup.members_covered, 1);
        assert!(rollup.members_covered <= rollup.members_total);
        assert_eq!(
            rollup.gaps.len() as u32,
            rollup.members_total - rollup.members_covered,
            "every uncovered member is named, never merely counted"
        );

        // The sums cover the one member that was read in full, and nothing else: the denied
        // member's readable pod count and the absent member's stale 100 nodes stay out.
        assert_eq!((rollup.node_ready, rollup.node_total), (2, 3));
        assert_eq!(rollup.unhealthy_pods, 4);

        let by_context: BTreeMap<&str, &FleetGapReason> = rollup
            .gaps
            .iter()
            .map(|g| (g.context.as_str(), &g.reason))
            .collect();

        // Four reasons, four shapes. A version that reduced them to one would fail here even
        // though the counts above would still add up.
        let FleetGapReason::NotPermitted { detail } = by_context["denied"] else {
            panic!("a 403 is NotPermitted, got {:?}", by_context["denied"]);
        };
        assert!(
            detail.contains("cannot list resource \"nodes\""),
            "the API server's own reason, verbatim: {detail}"
        );
        assert!(matches!(
            by_context["partly-denied"],
            FleetGapReason::NotPermitted { .. }
        ));
        let FleetGapReason::Unreachable { detail } = by_context["unreachable"] else {
            panic!("a connection failure is Unreachable");
        };
        assert!(detail.contains("tcp connect error"), "{detail}");
        assert_eq!(by_context["retired"], &FleetGapReason::Absent);
        assert_eq!(by_context["unpolled"], &FleetGapReason::Pending);

        // FR-011 explicitly: denied and absent never reduce to the same reason.
        assert_ne!(
            by_context["denied"], by_context["retired"],
            "a denied member must not read as an absent one"
        );
        assert_ne!(by_context["denied"], by_context["unreachable"]);
        assert_ne!(by_context["retired"], by_context["unpolled"]);
    }

    /// T032 (FR-015): the rollup answers from the cache `fleet::emit_summary` fills, so with an
    /// empty cache every present member is `Pending` — it does not go and poll one.
    #[test]
    fn an_empty_cache_yields_pending_not_a_poll() {
        let rollup = rollup_for(
            "prod",
            &[member("prod-a", FleetMemberAvailability::Present)],
            &BTreeMap::new(),
        );
        assert_eq!(rollup.members_covered, 0);
        assert_eq!(rollup.gaps[0].reason, FleetGapReason::Pending);
    }

    /// T032: a group whose *stored* selector no longer parses is reported as a parse error, not
    /// as a rollup over zero members (FR-005) — the two are different facts about the fleet and
    /// the UI must be able to tell the operator which one happened.
    #[test]
    fn a_group_with_an_unparseable_stored_selector_is_not_a_group_matching_nothing() {
        let broken = FleetFile {
            groups: BTreeMap::from([(
                "prod".to_owned(),
                FleetGroupRecord {
                    selector: "env in prod".to_owned(),
                    origin: SettingOrigin::Local,
                },
            )]),
            ..FleetFile::default()
        };
        let (_, events) = run(
            &broken,
            BackendCmd::RequestFleetGroupRollup {
                group: "prod".to_owned(),
            },
        );
        let Some(SystemEvent::FleetModelRejected { reason, field }) = events.last() else {
            panic!("expected a rejection, got {events:?}");
        };
        assert_eq!(field.as_deref(), Some("selector"));
        assert!(reason.contains("env in prod"), "{reason}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SystemEvent::FleetGroupRollupUpdated(_))),
            "no rollup may be emitted for a group that cannot be evaluated"
        );

        // And a group that does parse but matches nobody *does* get a rollup, saying so.
        let empty = FleetFile {
            groups: BTreeMap::from([(
                "prod".to_owned(),
                FleetGroupRecord {
                    selector: "env=nowhere".to_owned(),
                    origin: SettingOrigin::Local,
                },
            )]),
            ..FleetFile::default()
        };
        let (_, events) = run(
            &empty,
            BackendCmd::RequestFleetGroupRollup {
                group: "prod".to_owned(),
            },
        );
        let Some(SystemEvent::FleetGroupRollupUpdated(rollup)) = events.last() else {
            panic!("expected a rollup, got {events:?}");
        };
        assert_eq!(rollup.members_total, 0);
        assert!(rollup.gaps.is_empty());
    }

    /// T032: an unknown group name is a rejection naming `group`, distinct again from an empty
    /// rollup.
    #[test]
    fn an_unknown_group_is_rejected_by_name() {
        let (_, events) = run(
            &FleetFile::default(),
            BackendCmd::RequestFleetGroupRollup {
                group: "nope".to_owned(),
            },
        );
        let Some(SystemEvent::FleetModelRejected { reason, field }) = events.last() else {
            panic!("expected a rejection, got {events:?}");
        };
        assert_eq!(field.as_deref(), Some("group"));
        assert!(reason.contains("nope"), "{reason}");
    }

    // -----------------------------------------------------------------------------------
    // T041 / T042 (US3): sharing, origin resolution, and the never-delete rule.
    // -----------------------------------------------------------------------------------

    /// A throwaway state root whose `team.json` points at `store`. Keep the returned
    /// `TempDir` alive for the length of the test; dropping it deletes the state root.
    ///
    /// `set_thread_state_root` is thread-local and every test gets its own thread, so this
    /// is safe under the parallel runner — and it is set *before* `team.json` is written, so
    /// the config lands in the temp root and never in the operator's real `~/.fathom`.
    fn state_root_pointing_at(store: &Path) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::backend::persistence::set_thread_state_root(dir.path().to_path_buf());
        point_team_config_at(store);
        dir
    }

    fn point_team_config_at(store: &Path) {
        crate::backend::persistence::save_team_config(&crate::backend::persistence::TeamConfig {
            store_path: Some(store.display().to_string()),
            ..Default::default()
        })
        .unwrap();
    }

    /// Dispatch one command against the state root already in scope, and return the system
    /// events it emitted.
    fn dispatch(cmd: BackendCmd) -> Vec<SystemEvent> {
        let mut state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();
        assert!(
            handle(cmd, &mut state, &tx).unwrap().is_none(),
            "a 046 command must be claimed by this family"
        );
        rx.try_iter()
            .filter_map(|env| match env.event {
                BackendEvent::System(event) => Some(event),
                _ => None,
            })
            .collect()
    }

    /// `fleet.json` as it stands on disk, asserting it still parses.
    fn fleet_on_disk() -> FleetFile {
        let FleetLoad { file, failure } = crate::backend::persistence::load_fleet_model();
        assert_eq!(failure, None, "fleet.json must still be readable");
        file
    }

    fn definitions_event(
        events: &[SystemEvent],
    ) -> (Vec<SharedFleetDefinition>, Vec<AdoptedFleetDefinition>) {
        events
            .iter()
            .find_map(|e| match e {
                SystemEvent::SharedFleetDefinitionsUpdated {
                    definitions,
                    adopted,
                } => Some((definitions.clone(), adopted.clone())),
                _ => None,
            })
            .expect("SharedFleetDefinitionsUpdated")
    }

    fn store_unavailable_reason(events: &[SystemEvent]) -> String {
        events
            .iter()
            .find_map(|e| match e {
                SystemEvent::FleetStoreUnavailable { reason } => Some(reason.clone()),
                _ => None,
            })
            .expect("FleetStoreUnavailable")
    }

    fn share(display_name: Option<&str>, labels: &[(&str, &str)]) -> FleetMemberShare {
        FleetMemberShare {
            display_name: display_name.map(str::to_owned),
            labels: labels
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    /// Put a definition in the store the way a teammate's publish would, and hand back the
    /// row as stored (so a test can use the `rev` the store actually assigned).
    fn publish_into(
        store: &Path,
        id: &str,
        members: BTreeMap<String, FleetMemberShare>,
        groups: BTreeMap<String, String>,
    ) -> SharedFleetDefinition {
        let existing = team_store::read_fleet_definitions(store)
            .unwrap_or_default()
            .into_iter()
            .find(|d| d.id == id)
            .map_or(0, |d| d.rev);
        team_store::publish_fleet_definition(
            store,
            SharedFleetDefinition {
                id: id.to_owned(),
                members,
                groups,
                rev: existing,
                shared_by: "alice".to_owned(),
                updated_at: "2026-10-02T00:00:00Z".to_owned(),
                removed: false,
            },
            existing,
        )
        .unwrap();
        team_store::read_fleet_definitions(store)
            .unwrap()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap()
    }

    /// T041, FR-014: **no code path deletes an adopted copy.** Four ways to lose the shared
    /// side, one after another, and the adopted row is still there after every one of them.
    ///
    /// The three that the rule names — an unreachable store, an upstream `removed: true`,
    /// and a definition that is gone from the store entirely — are deliberately all here,
    /// because they are three different branches in two different functions and a version
    /// that deleted on any one of them would still pass a test that exercised the others.
    #[test]
    fn nothing_deletes_an_adopted_copy() {
        let store = tempfile::tempdir().unwrap();
        let _root = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&FleetFile::default()).unwrap();
        publish_into(
            store.path(),
            "prod",
            BTreeMap::from([(
                "prod-a".to_owned(),
                share(Some("prod a"), &[("env", "prod")]),
            )]),
            BTreeMap::from([("prod".to_owned(), "env in (prod)".to_owned())]),
        );

        dispatch(BackendCmd::AdoptFleetDefinition {
            id: "prod".to_owned(),
        });
        let adopted = fleet_on_disk().adopted;
        assert!(adopted.contains_key("prod"), "adoption records the copy");
        assert!(adopted["prod"].still_shared);

        // (1) Removed upstream. The store keeps the row, marked; the local copy keeps
        // everything and stops following.
        team_store::remove_fleet_definition(store.path(), "prod", adopted["prod"].definition.rev)
            .unwrap();
        dispatch(BackendCmd::RefreshSharedFleetDefinitions);
        let file = fleet_on_disk();
        assert!(
            file.adopted.contains_key("prod"),
            "upstream removal must not delete the copy"
        );
        assert!(!file.adopted["prod"].still_shared);
        assert_eq!(
            file.adopted["prod"].definition.groups["prod"], "env in (prod)",
            "the adopted copy keeps its contents, not just its key"
        );

        // (2) Gone from the store entirely — not even a `removed` row left to find.
        std::fs::remove_file(store.path().join("fleet-definitions.json")).unwrap();
        dispatch(BackendCmd::RefreshSharedFleetDefinitions);
        assert!(
            fleet_on_disk().adopted.contains_key("prod"),
            "a definition that vanished from the store must not delete the copy"
        );

        // (3) The store itself cannot be read. `check_reachable` creates a missing
        // directory, so the unreachable case is a path that exists and is not one.
        let blocked = store.path().join("not-a-directory");
        std::fs::write(&blocked, "x").unwrap();
        point_team_config_at(&blocked);
        let events = dispatch(BackendCmd::RefreshSharedFleetDefinitions);
        assert!(
            fleet_on_disk().adopted.contains_key("prod"),
            "an unreachable store must not delete the copy"
        );
        // FR-012: `StoreError`'s own message, not wording invented here.
        assert_eq!(
            store_unavailable_reason(&events),
            StoreError::Unreachable(format!(
                "{} exists but is not a directory",
                blocked.display()
            ))
            .to_string()
        );
        let (definitions, still_adopted) = definitions_event(&events);
        assert!(
            definitions.is_empty(),
            "an unreadable store offers nothing to adopt"
        );
        assert!(
            still_adopted
                .iter()
                .any(|a| a.definition.id == "prod" && !a.still_shared),
            "the adopted row is reported as not following, never omitted"
        );

        // (4) The operator withdraws it themselves, against a live store.
        point_team_config_at(store.path());
        let republished = publish_into(
            store.path(),
            "prod",
            BTreeMap::from([(
                "prod-a".to_owned(),
                share(Some("prod a"), &[("env", "prod")]),
            )]),
            BTreeMap::from([("prod".to_owned(), "env in (prod)".to_owned())]),
        );
        assert_eq!(
            republished.rev,
            fleet_on_disk().adopted["prod"].definition.rev
        );
        dispatch(BackendCmd::RemoveFleetDefinition {
            id: "prod".to_owned(),
        });
        let file = fleet_on_disk();
        assert!(
            file.adopted.contains_key("prod"),
            "un-publishing is not a delete, locally or upstream"
        );
        assert!(!file.adopted["prod"].still_shared);
        let rows = team_store::read_fleet_definitions(store.path()).unwrap();
        assert_eq!(rows.len(), 1, "the store row survives its own removal");
        assert!(rows[0].removed);
    }

    /// The same rule, read off the source: nothing in this module takes a key *out* of
    /// `adopted`. The behavioural test above covers the paths that exist today; this one
    /// fails the moment a fifth path is written that deletes instead of marking.
    ///
    /// The needles are assembled at runtime because a literal would match itself here.
    #[test]
    fn no_code_path_removes_an_adopted_definition() {
        let src = include_str!("fleet.rs");
        for verb in ["remove", "clear", "retain", "pop_first", "pop_last", "take"] {
            let needle = format!("adopted.{verb}");
            assert!(
                !src.contains(&needle),
                "`{needle}` would delete an adopted copy, which FR-014 forbids"
            );
        }
        assert!(
            !src.contains(&format!("adopted {} BTreeMap", "=")),
            "replacing the adopted map wholesale would delete adopted copies (FR-014)"
        );
    }

    /// T042, FR-013: a local value wins, says it is local, and the shared value it overrode
    /// stays readable — it lives in the adopted copy, which keeps the whole definition
    /// verbatim.
    #[test]
    fn a_local_value_wins_and_the_shared_value_it_overrode_stays_readable() {
        let store = tempfile::tempdir().unwrap();
        let _root = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&FleetFile {
            members: BTreeMap::from([(
                "prod-a".to_owned(),
                FleetMemberRecord {
                    display_name: Some("my prod".to_owned()),
                    labels: BTreeMap::from([
                        ("env".to_owned(), "prod-eu".to_owned()),
                        ("tier".to_owned(), "gold".to_owned()),
                    ]),
                    origins: BTreeMap::from([
                        ("display_name".to_owned(), SettingOrigin::Local),
                        ("labels/env".to_owned(), SettingOrigin::Local),
                    ]),
                },
            )]),
            groups: BTreeMap::from([(
                "prod".to_owned(),
                FleetGroupRecord {
                    selector: "env in (prod-eu)".to_owned(),
                    origin: SettingOrigin::Local,
                },
            )]),
            ..FleetFile::default()
        })
        .unwrap();
        publish_into(
            store.path(),
            "fleet",
            BTreeMap::from([(
                "prod-a".to_owned(),
                share(
                    Some("shared prod a"),
                    &[("env", "prod"), ("region", "eu-west-1")],
                ),
            )]),
            BTreeMap::from([
                ("prod".to_owned(), "env in (prod)".to_owned()),
                ("staging".to_owned(), "env in (staging)".to_owned()),
            ]),
        );

        let events = dispatch(BackendCmd::AdoptFleetDefinition {
            id: "fleet".to_owned(),
        });
        let file = fleet_on_disk();
        let record = &file.members["prod-a"];

        // The local values win and are still marked local.
        assert_eq!(record.display_name.as_deref(), Some("my prod"));
        assert_eq!(record.origins["display_name"], SettingOrigin::Local);
        assert_eq!(record.labels["env"], "prod-eu");
        assert_eq!(record.origins["labels/env"], SettingOrigin::Local);
        // A label only the operator has is untouched: adoption adds, it never prunes.
        assert_eq!(record.labels["tier"], "gold");
        // A shared label the operator has no opinion on arrives, marked as the shared value.
        // `SettingOrigin` has no `Shared`; `OrgDefault` *is* the shared value.
        assert_eq!(record.labels["region"], "eu-west-1");
        assert_eq!(record.origins["labels/region"], SettingOrigin::OrgDefault);
        // A locally-defined group wins exactly as a local label does; a shared-only group
        // arrives marked shared.
        assert_eq!(file.groups["prod"].selector, "env in (prod-eu)");
        assert_eq!(file.groups["prod"].origin, SettingOrigin::Local);
        assert_eq!(file.groups["staging"].origin, SettingOrigin::OrgDefault);

        // FR-013's second half: every shared value that lost is still readable.
        let adopted = &file.adopted["fleet"];
        let shared_member = &adopted.definition.members["prod-a"];
        assert_eq!(shared_member.display_name.as_deref(), Some("shared prod a"));
        assert_eq!(shared_member.labels["env"], "prod");
        assert_eq!(adopted.definition.groups["prod"], "env in (prod)");

        // And the event carries both sides, so the UI needs no second request to show them.
        let (definitions, reported) = definitions_event(&events);
        assert!(definitions.iter().any(|d| d.id == "fleet"));
        assert_eq!(reported.len(), 1);
        assert_eq!(
            reported[0].definition.members["prod-a"].labels["env"],
            "prod"
        );

        // The emitted model reports each value's origin for T043 to render.
        let members = match events
            .iter()
            .find(|e| matches!(e, SystemEvent::FleetModelUpdated { .. }))
            .expect("FleetModelUpdated")
        {
            SystemEvent::FleetModelUpdated { members, .. } => members.clone(),
            _ => unreachable!(),
        };
        let emitted = members.iter().find(|m| m.context == "prod-a").unwrap();
        assert_eq!(emitted.origins["display_name"], SettingOrigin::Local);
        assert_eq!(emitted.origins["labels/region"], SettingOrigin::OrgDefault);
    }

    /// T042, FR-014: a row that stopped being shared is **in** the event, not filtered out
    /// of it. Dropping it would look, to the operator, exactly like the deletion FR-014
    /// forbids — the copy sits in `fleet.json` while the UI stops showing it.
    #[test]
    fn the_definitions_event_keeps_rows_that_are_no_longer_shared() {
        let store = tempfile::tempdir().unwrap();
        let _root = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&FleetFile::default()).unwrap();
        let row = publish_into(
            store.path(),
            "prod",
            BTreeMap::from([("prod-a".to_owned(), share(None, &[("env", "prod")]))]),
            BTreeMap::new(),
        );
        dispatch(BackendCmd::AdoptFleetDefinition {
            id: "prod".to_owned(),
        });
        team_store::remove_fleet_definition(store.path(), "prod", row.rev).unwrap();

        let events = dispatch(BackendCmd::RefreshSharedFleetDefinitions);
        let (definitions, adopted) = definitions_event(&events);
        assert_eq!(
            adopted.len(),
            1,
            "the adopted row must be reported, with still_shared false"
        );
        assert!(!adopted[0].still_shared);
        assert_eq!(adopted[0].definition.id, "prod");
        assert!(
            definitions.iter().any(|d| d.id == "prod" && d.removed),
            "the withdrawn store row is reported as removed, not hidden"
        );
    }

    /// T040: publish, then adopt from a second install, reproduces the same group names and
    /// memberships — with no per-cluster filing step, because membership is computed from
    /// the labels that travelled (FR-004, FR-012).
    #[test]
    fn publish_then_adopt_reproduces_the_groups_and_memberships() {
        let store = tempfile::tempdir().unwrap();

        // Operator A publishes their whole model.
        let _a = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&file()).unwrap();
        dispatch(BackendCmd::PublishFleetDefinition {
            id: "fleet".to_owned(),
        });
        let rows = team_store::read_fleet_definitions(store.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].groups["prod"], "env in (prod,staging)");
        assert_eq!(
            rows[0].members["prod-a"].display_name.as_deref(),
            Some("prod a")
        );
        assert_eq!(rows[0].rev, 1);
        // Publishing records the publisher's own adoption, so their copy follows it.
        assert!(fleet_on_disk().adopted["fleet"].still_shared);

        // Operator B, with an empty model, adopts it.
        let _b = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&FleetFile::default()).unwrap();
        let events = dispatch(BackendCmd::AdoptFleetDefinition {
            id: "fleet".to_owned(),
        });
        let file = fleet_on_disk();
        assert_eq!(file.groups["prod"].selector, "env in (prod,staging)");
        assert_eq!(file.groups["prod"].origin, SettingOrigin::OrgDefault);
        assert_eq!(file.members["prod-a"].labels["env"], "prod");
        assert_eq!(
            file.members["prod-a"].origins["labels/env"],
            SettingOrigin::OrgDefault
        );
        // The group membership B sees is the one A's labels imply, resolved here.
        let members = match events
            .iter()
            .find(|e| matches!(e, SystemEvent::FleetModelUpdated { .. }))
            .expect("FleetModelUpdated")
        {
            SystemEvent::FleetModelUpdated { members, .. } => members.clone(),
            _ => unreachable!(),
        };
        let prod_a = members.iter().find(|m| m.context == "prod-a").unwrap();
        assert_eq!(prod_a.groups, vec!["prod".to_owned()]);
    }

    /// A publish with no store configured reports `StoreError`'s own words and writes
    /// nothing — and the definition id is still the operator's, not invented here.
    #[test]
    fn publishing_with_no_store_configured_reports_the_store_error() {
        let dir = tempfile::tempdir().unwrap();
        crate::backend::persistence::set_thread_state_root(dir.path().to_path_buf());
        crate::backend::persistence::save_fleet_model(&file()).unwrap();
        let events = dispatch(BackendCmd::PublishFleetDefinition {
            id: "fleet".to_owned(),
        });
        assert_eq!(
            store_unavailable_reason(&events),
            StoreError::Unreachable("no shared config store configured".to_owned()).to_string()
        );
        assert!(fleet_on_disk().adopted.is_empty());
    }

    /// Adopting an id the store does not hold is a rejection naming the id, not an empty
    /// adoption and not a store failure.
    #[test]
    fn adopting_a_definition_that_is_not_there_is_rejected_by_name() {
        let store = tempfile::tempdir().unwrap();
        let _root = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&FleetFile::default()).unwrap();
        let events = dispatch(BackendCmd::AdoptFleetDefinition {
            id: "nope".to_owned(),
        });
        let (reason, field) = events
            .iter()
            .find_map(|e| match e {
                SystemEvent::FleetModelRejected { reason, field } => {
                    Some((reason.clone(), field.clone()))
                }
                _ => None,
            })
            .expect("FleetModelRejected");
        assert!(reason.contains("nope"), "{reason}");
        assert_eq!(field.as_deref(), Some("id"));
        assert!(fleet_on_disk().adopted.is_empty());
    }

    /// A stale publish is a conflict, reported in `StoreError`'s own words, and the
    /// teammate's definition is left alone (FR-012).
    #[test]
    fn a_stale_publish_is_reported_as_a_conflict_and_overwrites_nothing() {
        let store = tempfile::tempdir().unwrap();
        let _root = state_root_pointing_at(store.path());
        crate::backend::persistence::save_fleet_model(&file()).unwrap();
        dispatch(BackendCmd::PublishFleetDefinition {
            id: "fleet".to_owned(),
        });
        // A teammate publishes over it; this operator's adopted copy is now stale.
        publish_into(
            store.path(),
            "fleet",
            BTreeMap::from([("prod-b".to_owned(), share(Some("bob's"), &[]))]),
            BTreeMap::new(),
        );
        let events = dispatch(BackendCmd::PublishFleetDefinition {
            id: "fleet".to_owned(),
        });
        assert!(
            store_unavailable_reason(&events).starts_with("conflict: expected rev"),
            "{events:?}"
        );
        let rows = team_store::read_fleet_definitions(store.path()).unwrap();
        assert!(
            rows[0].members.contains_key("prod-b"),
            "the teammate's definition must survive a stale publish"
        );
        // A conflict is not a lost store: the adopted copy keeps following.
        assert!(fleet_on_disk().adopted["fleet"].still_shared);
    }

    /// Principle VI: an unparseable `fleet.json` is reported and left alone, even when the
    /// command that found it was about the shared store.
    #[test]
    fn an_unreadable_fleet_file_is_never_overwritten_by_a_sharing_command() {
        let store = tempfile::tempdir().unwrap();
        let _root = state_root_pointing_at(store.path());
        let path = crate::backend::persistence::fleet_file_path().unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        for cmd in [
            BackendCmd::RefreshSharedFleetDefinitions,
            BackendCmd::PublishFleetDefinition {
                id: "fleet".to_owned(),
            },
            BackendCmd::RemoveFleetDefinition {
                id: "fleet".to_owned(),
            },
        ] {
            let events = dispatch(cmd);
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, SystemEvent::FleetModelLoadFailed { .. })),
                "{events:?}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        }
    }
}

//! Teams & Organizations (014): Cluster Settings (§10, local + org-sync) and shared
//! team connection profiles. All persistence and shared-store I/O happens here — the UI
//! never reads/writes `~/.fathom/` or the shared store directly (Constitution Principle II).

use std::path::PathBuf;

use crossbeam_channel::Sender;

use crate::backend::events;
use crate::backend::idp_directory::{self, FathomRole};
use crate::backend::persistence::{self, TeamConfig};
use crate::backend::state::BackendState;
use crate::backend::team_store::{self, StoreError};
use crate::protocol::{
    AdoptedConnectionProfile, BackendCmd, BackendEvent, BackendEventEnvelope, ClusterSettings,
    ConnectionProfile, Correlation, OrgClusterSettings, Route, SettingOrigin, SystemEvent,
    TeamLink, TeamPublishRejection, TeamStoreStatus, scan_fields_for_secrets, section_keys,
};

/// Handles every 014 command; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::LoadClusterSettings { context } => {
            load_cluster_settings(state, sender, context)
        }
        BackendCmd::SaveClusterSettings { context, settings } => {
            save_cluster_settings(state, sender, context, settings)
        }
        BackendCmd::SetTeamStorePath { path } => set_team_store_path(state, sender, path),
        BackendCmd::RefreshTeamStoreStatus => refresh_team_store_status(state, sender),
        BackendCmd::RefreshConnectionProfiles => refresh_connection_profiles(state, sender),
        BackendCmd::PublishConnectionProfile {
            context_alias,
            api_server,
            default_namespace,
        } => {
            if gate(state, sender, FathomRole::Editor) {
                publish_connection_profile(
                    state,
                    sender,
                    context_alias,
                    api_server,
                    default_namespace,
                )
            }
        }
        BackendCmd::AdoptConnectionProfile { profile_id } => {
            adopt_connection_profile(state, sender, profile_id)
        }
        BackendCmd::RemoveConnectionProfile { profile_id } => {
            remove_connection_profile(state, sender, profile_id)
        }
        BackendCmd::PublishOrgClusterSettings { context } => {
            if gate(state, sender, FathomRole::Editor) {
                publish_org_cluster_settings(state, sender, context)
            }
        }
        BackendCmd::AcceptOrgClusterSettings { context } => {
            if gate(state, sender, FathomRole::Viewer) {
                accept_org_cluster_settings(state, sender, context)
            }
        }
        BackendCmd::LinkTeamProfile { group, profile_id } => {
            if gate(state, sender, FathomRole::Admin) {
                change_link(state, sender, TeamLink { group, profile_id }, true)
            }
        }
        BackendCmd::UnlinkTeamProfile { group, profile_id } => {
            if gate(state, sender, FathomRole::Admin) {
                change_link(state, sender, TeamLink { group, profile_id }, false)
            }
        }
        BackendCmd::DismissOrgClusterSettings { context } => {
            dismiss_org_cluster_settings(state, sender, context)
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

/// 033 US3 (R9): the Fathom role map gates team publish/sync/link. Kubeconfig-only contexts
/// (no OIDC identity in play) are unrestricted. Refusals are surfaced, never silent.
fn gate(state: &BackendState, sender: &Sender<BackendEventEnvelope>, required: FathomRole) -> bool {
    let identity = super::idp::active_identity(state);
    let map = persistence::load_oidc_config().role_map;
    match idp_directory::require_role(identity.as_ref(), &map, required) {
        Ok(()) => true,
        Err(reason) => {
            emit_rejection(
                state,
                sender,
                TeamPublishRejection::RoleForbidden { reason },
            );
            false
        }
    }
}

fn emit_links(state: &BackendState, sender: &Sender<BackendEventEnvelope>, links: Vec<TeamLink>) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::team_profiles()),
        Correlation::None,
        BackendEvent::System(SystemEvent::TeamLinksUpdated { links }),
    );
}

fn change_link(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    link: TeamLink,
    add: bool,
) {
    let config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: "no shared config store configured".to_owned(),
            },
        );
        return;
    };
    let result = if add {
        team_store::link_team(&root, link)
    } else {
        team_store::unlink_team(&root, &link)
    };
    match result {
        Ok(links) => emit_links(state, sender, links),
        Err(StoreError::Invalid(reason)) => emit_rejection(
            state,
            sender,
            TeamPublishRejection::SuspectedSecret {
                field: "teamLink".to_owned(),
                reason,
            },
        ),
        Err(e) => emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: e.to_string(),
            },
        ),
    }
}

fn current_operator() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "operator".to_owned())
}

fn now_rfc3339() -> String {
    // A UTC "seconds"-precision RFC3339 stamp without adding a chrono dependency
    // dedicated to this feature (chrono is already a workspace dependency elsewhere,
    // but a hand-rolled stamp keeps this module dependency-free and trivially testable).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Days-since-epoch -> Y-M-D (civil_from_days, Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn route() -> Route {
    Route::global(section_keys::cluster_settings())
}

fn emit_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    settings: ClusterSettings,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(),
        Correlation::None,
        BackendEvent::System(SystemEvent::ClusterSettingsLoaded { context, settings }),
    );
}

fn emit_settings_error(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    reason: String,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(),
        Correlation::None,
        BackendEvent::System(SystemEvent::ClusterSettingsError { context, reason }),
    );
}

fn emit_rejection(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    rejection: TeamPublishRejection,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::team_profiles()),
        Correlation::None,
        BackendEvent::System(SystemEvent::TeamPublishRejected { rejection }),
    );
}

fn emit_profiles(state: &BackendState, sender: &Sender<BackendEventEnvelope>, config: &TeamConfig) {
    emit_profiles_with_available(state, sender, config, Vec::new());
}

fn emit_profiles_with_available(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    config: &TeamConfig,
    available: Vec<ConnectionProfile>,
) {
    let adopted: Vec<AdoptedConnectionProfile> = config.adopted.values().cloned().collect();
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::team_profiles()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ConnectionProfilesUpdated { adopted, available }),
    );
}

fn emit_store_status(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    config: &TeamConfig,
) {
    let status = match &config.store_path {
        None => TeamStoreStatus {
            store_path: None,
            reachable: false,
            error: None,
        },
        Some(path) => match team_store::check_reachable(&PathBuf::from(path)) {
            Ok(()) => TeamStoreStatus {
                store_path: Some(path.clone()),
                reachable: true,
                error: None,
            },
            Err(e) => TeamStoreStatus {
                store_path: Some(path.clone()),
                reachable: false,
                error: Some(e.to_string()),
            },
        },
    };
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::team_profiles()),
        Correlation::None,
        BackendEvent::System(SystemEvent::TeamStoreStatusUpdated(status)),
    );
}

fn store_root(config: &TeamConfig) -> Option<PathBuf> {
    config.store_path.as_ref().map(PathBuf::from)
}

// -- Cluster Settings (local, §10) -------------------------------------------------------

fn load_cluster_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) {
    let settings = persistence::load_cluster_settings(&context);
    emit_settings(state, sender, context, settings);
}

fn save_cluster_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    settings: ClusterSettings,
) {
    match persistence::save_cluster_settings(&context, &settings) {
        Ok(()) => emit_settings(state, sender, context, settings),
        Err(e) => emit_settings_error(
            state,
            sender,
            context,
            format!("Failed to save cluster settings: {e}"),
        ),
    }
}

// -- Team store configuration / status ---------------------------------------------------

fn set_team_store_path(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    path: Option<String>,
) {
    let mut config = persistence::load_team_config();
    config.store_path = path;
    if let Err(e) = persistence::save_team_config(&config) {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to save team store path: {e}"),
        );
        return;
    }
    emit_store_status(state, sender, &config);
    emit_profiles(state, sender, &config);
}

fn refresh_team_store_status(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let config = persistence::load_team_config();
    emit_store_status(state, sender, &config);
}

// -- Connection profiles (US1) -----------------------------------------------------------

fn refresh_connection_profiles(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let mut config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        emit_profiles(state, sender, &config);
        return;
    };
    match team_store::read_profiles(&root) {
        Ok(shared) => {
            // Reconcile: an adopted profile whose upstream entry is now `removed` (or
            // gone entirely) is marked `still_shared = false`, never deleted (FR-004,
            // Constitution Principle VI). A shared profile the operator hasn't adopted
            // yet is not added here — adoption is explicit (FR-003).
            for (id, adopted) in config.adopted.iter_mut() {
                match shared.iter().find(|p| &p.id == id) {
                    Some(p) if !p.removed => {
                        adopted.profile = p.clone();
                        adopted.still_shared = true;
                    }
                    _ => adopted.still_shared = false,
                }
            }
            let _ = persistence::save_team_config(&config);
            emit_store_status(state, sender, &config);
            if let Ok(links) = team_store::read_links(&root) {
                emit_links(state, sender, links);
            }
            let available: Vec<ConnectionProfile> = shared
                .into_iter()
                .filter(|p| !p.removed && !config.adopted.contains_key(&p.id))
                .collect();
            emit_profiles_with_available(state, sender, &config, available);
        }
        Err(e) => {
            emit_store_status(state, sender, &config);
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Team profile sync unavailable: {e}"),
            );
            // Already-adopted local profiles still work; just report them as-is (FR-005).
            emit_profiles(state, sender, &config);
        }
    }
}

fn publish_connection_profile(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context_alias: String,
    api_server: String,
    default_namespace: Option<String>,
) {
    if let Err((field, reason)) = scan_fields_for_secrets(&[
        ("contextAlias", context_alias.as_str()),
        ("apiServer", api_server.as_str()),
        (
            "defaultNamespace",
            default_namespace.as_deref().unwrap_or(""),
        ),
    ]) {
        emit_rejection(
            state,
            sender,
            TeamPublishRejection::SuspectedSecret { field, reason },
        );
        return;
    }

    let mut config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: "no shared config store configured".to_owned(),
            },
        );
        return;
    };

    let id = format!("{context_alias}-{}", api_server_fingerprint(&api_server));
    let profile = ConnectionProfile {
        id: id.clone(),
        context_alias,
        api_server,
        default_namespace,
        rev: 0,
        shared_by: current_operator(),
        updated_at: now_rfc3339(),
        removed: false,
    };
    let expected_rev = team_store::read_profiles(&root)
        .ok()
        .and_then(|profiles| profiles.iter().find(|p| p.id == id).map(|p| p.rev))
        .unwrap_or(0);

    match team_store::publish_profile(&root, profile.clone(), expected_rev) {
        Ok(rev) => {
            let mut published = profile;
            published.rev = rev;
            config.adopted.insert(
                id,
                AdoptedConnectionProfile {
                    profile: published,
                    adopted_at: now_rfc3339(),
                    still_shared: true,
                },
            );
            let _ = persistence::save_team_config(&config);
            emit_profiles(state, sender, &config);
        }
        Err(StoreError::Conflict {
            expected_rev,
            current_rev,
        }) => emit_rejection(
            state,
            sender,
            TeamPublishRejection::Conflict {
                expected_rev,
                current_rev,
            },
        ),
        Err(e) => emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: e.to_string(),
            },
        ),
    }
}

fn api_server_fingerprint(api_server: &str) -> String {
    // Stable, filesystem/JSON-key-safe id suffix. Not cryptographic — collision-avoidance
    // only, for readable, deterministic profile ids across a team.
    let mut hash: u64 = 1469598103934665603; // FNV-1a offset basis
    for b in api_server.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

fn adopt_connection_profile(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    profile_id: String,
) {
    let mut config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: "no shared config store configured".to_owned(),
            },
        );
        return;
    };
    match team_store::read_profiles(&root) {
        Ok(profiles) => match profiles
            .into_iter()
            .find(|p| p.id == profile_id && !p.removed)
        {
            Some(profile) => {
                config.adopted.insert(
                    profile_id,
                    AdoptedConnectionProfile {
                        profile,
                        adopted_at: now_rfc3339(),
                        still_shared: true,
                    },
                );
                let _ = persistence::save_team_config(&config);
                emit_profiles(state, sender, &config);
            }
            None => events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("No shared connection profile '{profile_id}' to adopt"),
            ),
        },
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Team profile sync unavailable: {e}"),
        ),
    }
}

fn remove_connection_profile(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    profile_id: String,
) {
    let mut config = persistence::load_team_config();
    // Removing this operator's local adoption never requires the store to be reachable —
    // it's purely local bookkeeping. Unpublishing upstream additionally needs the store.
    if let Some(root) = store_root(&config)
        && let Some(adopted) = config.adopted.get(&profile_id)
    {
        let _ = team_store::remove_profile(&root, &profile_id, adopted.profile.rev);
    }
    config.adopted.remove(&profile_id);
    let _ = persistence::save_team_config(&config);
    emit_profiles(state, sender, &config);
}

// -- Org-level Cluster Settings sync (US2) -----------------------------------------------

fn publish_org_cluster_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) {
    let config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: "no shared config store configured".to_owned(),
            },
        );
        return;
    };
    // FR-010: only offered for a shared connection; refuse otherwise rather than trusting
    // a UI that shouldn't have offered the affordance in the first place.
    let is_shared_context = config
        .adopted
        .values()
        .any(|a| a.profile.context_alias == context && a.still_shared);
    if !is_shared_context {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!(
                "'{context}' is not a shared team connection; nothing to publish org settings to"
            ),
        );
        return;
    }

    let local = persistence::load_cluster_settings(&context);
    if let Err((field, reason)) = scan_fields_for_secrets(&[
        ("prometheusPath", local.prometheus_path.as_str()),
        ("pathPrefix", local.path_prefix.as_str()),
    ]) {
        emit_rejection(
            state,
            sender,
            TeamPublishRejection::SuspectedSecret { field, reason },
        );
        return;
    }

    let expected_rev = team_store::read_org_cluster_settings(&root, &context)
        .ok()
        .flatten()
        .map_or(0, |s| s.rev);
    let payload = OrgClusterSettings {
        context: context.clone(),
        rev: expected_rev,
        published_by: current_operator(),
        published_at: now_rfc3339(),
        metrics_source: local.metrics_source,
        prometheus_path: local.prometheus_path,
        https: local.https,
        path_prefix: local.path_prefix,
        hidden_metrics: local.hidden_metrics,
    };
    match team_store::publish_org_cluster_settings(&root, payload, expected_rev) {
        Ok(_rev) => events::notify_info(
            state.cluster.seq.as_ref(),
            sender,
            format!("Published org-level Cluster Settings for '{context}'"),
        ),
        Err(StoreError::Conflict {
            expected_rev,
            current_rev,
        }) => emit_rejection(
            state,
            sender,
            TeamPublishRejection::Conflict {
                expected_rev,
                current_rev,
            },
        ),
        Err(e) => emit_rejection(
            state,
            sender,
            TeamPublishRejection::StoreUnreachable {
                reason: e.to_string(),
            },
        ),
    }
}

fn accept_org_cluster_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) {
    let config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        emit_settings_error(
            state,
            sender,
            context,
            "no shared config store configured".to_owned(),
        );
        return;
    };
    match team_store::read_org_cluster_settings(&root, &context) {
        Ok(Some(org)) => {
            // Explicit accept (FR-007): every field the operator hasn't locally
            // overridden follows the org default; fields already `Local` stay put
            // (FR-008) unless this *is* the accept action for that field, which the UI
            // only offers per-field-or-all — applying every field here matches "accept
            // the incoming change" from FR-007's UI flow; a per-field partial-accept is
            // just this same command called once the UI marks that field as no longer
            // locally overridden before calling it (kept as a UI-level concern).
            let mut local = persistence::load_cluster_settings(&context);
            local.metrics_source = org.metrics_source;
            local.prometheus_path = org.prometheus_path;
            local.https = org.https;
            local.path_prefix = org.path_prefix;
            local.hidden_metrics = org.hidden_metrics;
            local.org_rev_known = Some(org.rev);
            local.field_origin = crate::protocol::ClusterSettingsFieldOrigin {
                metrics_source: SettingOrigin::OrgDefault,
                prometheus_path: SettingOrigin::OrgDefault,
                https: SettingOrigin::OrgDefault,
                path_prefix: SettingOrigin::OrgDefault,
                hidden_metrics: SettingOrigin::OrgDefault,
            };
            match persistence::save_cluster_settings(&context, &local) {
                Ok(()) => {
                    let mut team_config = config;
                    team_config
                        .org_settings_rev_seen
                        .insert(context.clone(), org.rev);
                    let _ = persistence::save_team_config(&team_config);
                    emit_settings(state, sender, context, local);
                }
                Err(e) => emit_settings_error(
                    state,
                    sender,
                    context,
                    format!("Failed to apply org settings: {e}"),
                ),
            }
        }
        Ok(None) => emit_settings_error(
            state,
            sender,
            context,
            "no org-level settings published yet".to_owned(),
        ),
        Err(e) => emit_settings_error(
            state,
            sender,
            context,
            format!("Failed to read org settings: {e}"),
        ),
    }
}

fn dismiss_org_cluster_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) {
    let mut config = persistence::load_team_config();
    let Some(root) = store_root(&config) else {
        return;
    };
    if let Ok(Some(org)) = team_store::read_org_cluster_settings(&root, &context) {
        config
            .org_settings_rev_seen
            .insert(context.clone(), org.rev);
        let _ = persistence::save_team_config(&config);
    }
    // Local settings are untouched (FR-008): dismissing just stops the prompt.
    let local = persistence::load_cluster_settings(&context);
    emit_settings(state, sender, context, local);
}

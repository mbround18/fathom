//! What the Updates page and the stack page show: core types to wire types, reasons in words,
//! and the `UpdatesView` (contracts/protocol.md).

use std::collections::BTreeSet;

use depths_core::updates::labels as core_labels;
use depths_core::updates::policy as core;
use depths_core::updates::reference::ImageRef;
use depths_protocol as wire;
use depths_protocol::{RegistryStatus, ServiceUpdate, UpdateState, UpdatesView};

use super::Updates;
use super::creds::CredsSnapshot;

pub fn policy_to_core(p: wire::UpdatePolicy) -> core::UpdatePolicy {
    match p {
        wire::UpdatePolicy::Off => core::UpdatePolicy::Off,
        wire::UpdatePolicy::Notify => core::UpdatePolicy::Notify,
        wire::UpdatePolicy::Auto => core::UpdatePolicy::Auto,
    }
}

fn policy_to_wire(p: core::UpdatePolicy) -> wire::UpdatePolicy {
    match p {
        core::UpdatePolicy::Off => wire::UpdatePolicy::Off,
        core::UpdatePolicy::Notify => wire::UpdatePolicy::Notify,
        core::UpdatePolicy::Auto => wire::UpdatePolicy::Auto,
    }
}

fn rule_to_wire(r: core_labels::LabelRule) -> wire::LabelRule {
    match r {
        core_labels::LabelRule::Enable => wire::LabelRule::Enable,
        core_labels::LabelRule::MonitorOnly => wire::LabelRule::MonitorOnly,
        core_labels::LabelRule::Scope => wire::LabelRule::Scope,
        core_labels::LabelRule::NoPull => wire::LabelRule::NoPull,
    }
}

pub fn exclusion_to_wire(e: &core::Exclusion) -> wire::Exclusion {
    match e {
        core::Exclusion::PinnedByDigest => wire::Exclusion::PinnedByDigest,
        core::Exclusion::BuildContext => wire::Exclusion::BuildContext,
        core::Exclusion::LocalImage => wire::Exclusion::LocalImage,
        core::Exclusion::BadReference => wire::Exclusion::BadReference,
        core::Exclusion::LabelDisabled => wire::Exclusion::LabelDisabled,
        core::Exclusion::LabelOnlyMode => wire::Exclusion::LabelOnlyMode,
        core::Exclusion::ScopeMismatch { scope } => wire::Exclusion::ScopeMismatch {
            scope: scope.clone(),
        },
        core::Exclusion::NoPull => wire::Exclusion::NoPull,
        core::Exclusion::DepthsItself => wire::Exclusion::DepthsItself,
    }
}

pub fn effective_to_wire(e: &core::EffectivePolicy) -> wire::EffectivePolicy {
    wire::EffectivePolicy {
        policy: policy_to_wire(e.policy),
        source: match e.source {
            core::PolicySource::StackPolicy => wire::PolicySource::StackPolicy,
            core::PolicySource::Default => wire::PolicySource::Default,
            core::PolicySource::Label(rule) => wire::PolicySource::Label {
                rule: rule_to_wire(rule),
            },
        },
        excluded: e.excluded.as_ref().map(exclusion_to_wire),
        labels: e
            .labels
            .iter()
            .map(|l| wire::WatchtowerLabel {
                key: l.key.clone(),
                value: l.value.clone(),
                effect: match &l.effect {
                    core_labels::LabelEffect::Applied => wire::LabelEffect::Applied,
                    core_labels::LabelEffect::NotRun { reason } => wire::LabelEffect::NotRun {
                        reason: reason.clone(),
                    },
                    core_labels::LabelEffect::NotUsed { reason } => wire::LabelEffect::NotUsed {
                        reason: reason.clone(),
                    },
                    core_labels::LabelEffect::Unrecognised => wire::LabelEffect::Unrecognised,
                },
            })
            .collect(),
    }
}

/// Why a service is not checked, in words.
pub fn exclusion_reason(e: &core::Exclusion) -> String {
    match e {
        core::Exclusion::PinnedByDigest => {
            "pinned by digest: change the digest in the compose file to update".into()
        }
        core::Exclusion::BuildContext => "built from a build context, not pulled".into(),
        core::Exclusion::LocalImage => "the image exists only on this host".into(),
        core::Exclusion::BadReference => "the image reference can't be read".into(),
        core::Exclusion::LabelDisabled => "a Watchtower label turns updates off".into(),
        core::Exclusion::LabelOnlyMode => {
            "label-only mode is on and the service has no enable label".into()
        }
        core::Exclusion::ScopeMismatch { scope } => match scope {
            Some(s) => format!("its Watchtower scope `{s}` is not Depths' scope"),
            None => "it has no Watchtower scope and Depths has one".into(),
        },
        core::Exclusion::NoPull => "a Watchtower no-pull label".into(),
        core::Exclusion::DepthsItself => {
            "Depths' own container: update it by hand (docker compose pull, then docker compose up -d, where Depths is deployed)".into()
        }
    }
}

/// The registry hosts in the services, with their auth and backoff.
fn registries(u: &Updates, services: &[ServiceUpdate]) -> Vec<RegistryStatus> {
    let observed = u.store().observed();
    let creds = CredsSnapshot::load(&u.config().docker_config);
    let mut hosts: BTreeSet<String> = services
        .iter()
        .filter(|s| s.effective.excluded.is_none())
        .filter_map(|s| ImageRef::parse(&s.reference).ok())
        .map(|r| r.registry)
        .collect();
    hosts.extend(observed.backoff.keys().cloned());
    hosts
        .into_iter()
        .map(|host| RegistryStatus {
            auth: creds.auth_for(&host),
            backoff_until: observed.backoff.get(&host).map(|b| b.until.clone()),
            host,
        })
        .collect()
}

/// The panel note when `config.json` could not be read.
pub fn creds_note(u: &Updates) -> Option<String> {
    CredsSnapshot::load(&u.config().docker_config)
        .note()
        .map(str::to_owned)
}

/// Every known service, by stack then service.
pub fn services(u: &Updates) -> Vec<ServiceUpdate> {
    u.store().observed().services.into_values().collect()
}

/// A stack's services, for `StackDetail.updates`.
pub fn stack_services(u: &Updates, stack: &str) -> Vec<ServiceUpdate> {
    services(u)
        .into_iter()
        .filter(|s| s.stack == stack)
        .collect()
}

/// Stacks with at least one service whose update is available.
pub fn badge(services: &[ServiceUpdate]) -> u32 {
    let stacks: BTreeSet<&str> = services
        .iter()
        .filter(|s| s.state == UpdateState::Available)
        .map(|s| s.stack.as_str())
        .collect();
    u32::try_from(stacks.len()).unwrap_or(u32::MAX)
}

/// The Updates page.
pub fn updates_view(u: &Updates) -> UpdatesView {
    let decisions = u.store().decisions().ok();
    let settings = u.store().settings();
    let services = services(u);
    let auto = u.auto_status();
    let seen = u.watchtower();
    let acks = decisions
        .as_ref()
        .map(|d| d.watchtower_acks.clone())
        .unwrap_or_default();
    let observed = u.store().observed();
    let next_check = if u.checks_enabled() {
        super::scheduler::next_check(u)
    } else {
        None
    };
    UpdatesView {
        settings,
        available: u.availability(),
        zone: crate::config::host_zone_name(
            std::env::var("TZ").ok(),
            std::path::Path::new("/etc/localtime"),
        ),
        next_check: next_check.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        next_window: auto.next_window,
        // The live answer, so an acknowledgement lifts the banner before the next tick.
        auto_paused: super::auto::paused(u),
        registries: registries(u, &services),
        services,
        policies: decisions
            .as_ref()
            .map(|d| {
                d.policies
                    .iter()
                    .map(|(k, p)| (k.clone(), p.to_wire()))
                    .collect()
            })
            .unwrap_or_default(),
        recent_runs: observed.journal.iter().rev().take(10).cloned().collect(),
        watchtower: super::watchtower::view(&seen, &acks),
        paused_policies: auto.paused_policies,
    }
}

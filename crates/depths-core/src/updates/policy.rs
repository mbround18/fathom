//! The effective update policy of a service (data-model.md "Effective policy").
//!
//! Restriction rank, most restrictive first: excluded, `Off`, `Notify`, `Auto`. The most
//! restrictive of the stack policy, the reference and the labels wins, and the source is named.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::labels::{self, LabelRule, LabelSettings, WatchtowerLabel};
use super::reference::ImageRef;

/// What an administrator chose for a stack. Ordered by how much it allows: `Off < Notify < Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    Off,
    Notify,
    Auto,
}

impl UpdatePolicy {
    /// A stack with no stored entry (FR-012).
    pub const DEFAULT: UpdatePolicy = UpdatePolicy::Notify;
}

/// What decided the effective policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    StackPolicy,
    Default,
    Label(LabelRule),
}

impl fmt::Display for PolicySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StackPolicy => f.write_str("stack_policy"),
            Self::Default => f.write_str("default"),
            Self::Label(rule) => write!(f, "label:{}", rule.key()),
        }
    }
}

/// Why a service is never checked or updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exclusion {
    PinnedByDigest,
    BuildContext,
    LocalImage,
    BadReference,
    LabelDisabled,
    LabelOnlyMode,
    /// The container's own scope label, if it has one.
    ScopeMismatch {
        scope: Option<String>,
    },
    NoPull,
    DepthsItself,
}

impl Exclusion {
    /// Stable wire code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PinnedByDigest => "pinned_by_digest",
            Self::BuildContext => "build_context",
            Self::LocalImage => "local_image",
            Self::BadReference => "bad_reference",
            Self::LabelDisabled => "label_disabled",
            Self::LabelOnlyMode => "label_only_mode",
            Self::ScopeMismatch { .. } => "scope_mismatch",
            Self::NoPull => "no_pull",
            Self::DepthsItself => "depths_itself",
        }
    }
}

/// The policy that applies to one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectivePolicy {
    /// After labels: never less restrictive than the stack's. `Off` when excluded.
    pub policy: UpdatePolicy,
    pub source: PolicySource,
    pub excluded: Option<Exclusion>,
    /// Every Watchtower label seen, with what Depths did with it.
    pub labels: Vec<WatchtowerLabel>,
}

impl EffectivePolicy {
    /// Checked against the registry at all.
    pub fn is_checked(&self) -> bool {
        self.excluded.is_none() && self.policy != UpdatePolicy::Off
    }

    /// May be updated by a scheduled run.
    pub fn allows_auto(&self) -> bool {
        self.excluded.is_none() && self.policy == UpdatePolicy::Auto
    }
}

/// What the server knows about a service, beyond its labels.
#[derive(Debug, Clone, Copy)]
pub struct ServiceFacts<'a> {
    /// The compose `image:`, if any.
    pub image: Option<&'a str>,
    /// The service has a `build:` section.
    pub has_build: bool,
    /// The image exists only locally (no registry digest).
    pub local_image: bool,
    /// The container Depths itself runs in (FR-024).
    pub is_depths: bool,
    pub labels: &'a BTreeMap<String, String>,
}

/// The effective policy of a service. Exclusions from the service itself come first (Depths,
/// a build context, the reference), then the label rules of contracts/watchtower-labels.md.
pub fn effective(
    facts: ServiceFacts<'_>,
    stack_policy: Option<UpdatePolicy>,
    settings: LabelSettings<'_>,
) -> EffectivePolicy {
    let from_labels = labels::effective(facts.labels, stack_policy, settings);
    let own = if facts.is_depths {
        Some(Exclusion::DepthsItself)
    } else if facts.has_build {
        Some(Exclusion::BuildContext)
    } else {
        match facts.image.map(ImageRef::parse) {
            None | Some(Err(_)) => Some(Exclusion::BadReference),
            Some(Ok(r)) if r.is_pinned() => Some(Exclusion::PinnedByDigest),
            Some(Ok(_)) if facts.local_image => Some(Exclusion::LocalImage),
            Some(Ok(_)) => None,
        }
    };
    match own {
        Some(excluded) => EffectivePolicy {
            policy: UpdatePolicy::Off,
            source: stack_source(stack_policy),
            excluded: Some(excluded),
            labels: from_labels.labels,
        },
        None => from_labels,
    }
}

pub(crate) fn stack_source(stack_policy: Option<UpdatePolicy>) -> PolicySource {
    if stack_policy.is_some() {
        PolicySource::StackPolicy
    } else {
        PolicySource::Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn facts<'a>(image: Option<&'a str>, labels: &'a BTreeMap<String, String>) -> ServiceFacts<'a> {
        ServiceFacts {
            image,
            has_build: false,
            local_image: false,
            is_depths: false,
            labels,
        }
    }

    fn none() -> LabelSettings<'static> {
        LabelSettings {
            label_only: false,
            scope: None,
        }
    }

    #[test]
    fn no_entry_is_notify_from_default() {
        let l = BTreeMap::new();
        let p = effective(facts(Some("nginx"), &l), None, none());
        assert_eq!(
            (p.policy, p.source, &p.excluded),
            (UpdatePolicy::Notify, PolicySource::Default, &None)
        );
        assert!(p.is_checked() && !p.allows_auto());
    }

    #[test]
    fn stack_policy_is_named() {
        let l = BTreeMap::new();
        let p = effective(facts(Some("nginx"), &l), Some(UpdatePolicy::Auto), none());
        assert_eq!(
            (p.policy, p.source),
            (UpdatePolicy::Auto, PolicySource::StackPolicy)
        );
        assert!(p.allows_auto());
        let off = effective(facts(Some("nginx"), &l), Some(UpdatePolicy::Off), none());
        assert!(!off.is_checked());
    }

    #[test]
    fn rank_orders_policies() {
        assert!(
            UpdatePolicy::Off < UpdatePolicy::Notify && UpdatePolicy::Notify < UpdatePolicy::Auto
        );
    }

    #[test]
    fn labels_can_only_restrict() {
        let mut l = BTreeMap::new();
        l.insert(
            format!("{}.monitor-only", labels::PREFIX),
            "true".to_owned(),
        );
        let p = effective(facts(Some("nginx"), &l), Some(UpdatePolicy::Auto), none());
        assert_eq!(
            (p.policy, p.source),
            (
                UpdatePolicy::Notify,
                PolicySource::Label(LabelRule::MonitorOnly)
            )
        );
        assert_eq!(p.source.to_string(), "label:monitor-only");
    }

    #[test]
    fn every_exclusion() {
        let empty = BTreeMap::new();
        let pinned = format!("nginx@{D}");
        let auto = Some(UpdatePolicy::Auto);
        let cases: Vec<(ServiceFacts<'_>, Exclusion)> = vec![
            (facts(Some(&pinned), &empty), Exclusion::PinnedByDigest),
            (
                ServiceFacts {
                    has_build: true,
                    ..facts(Some("nginx"), &empty)
                },
                Exclusion::BuildContext,
            ),
            (
                ServiceFacts {
                    local_image: true,
                    ..facts(Some("myapp"), &empty)
                },
                Exclusion::LocalImage,
            ),
            (facts(Some("Nginx"), &empty), Exclusion::BadReference),
            (facts(None, &empty), Exclusion::BadReference),
            (
                ServiceFacts {
                    is_depths: true,
                    ..facts(Some("depths:dev"), &empty)
                },
                Exclusion::DepthsItself,
            ),
        ];
        for (f, want) in cases {
            let p = effective(f, auto, none());
            assert_eq!(p.excluded.as_ref(), Some(&want), "{want:?}");
            assert_eq!(p.policy, UpdatePolicy::Off);
            assert!(!p.is_checked() && !p.allows_auto());
        }

        let label = |k: &str, v: &str| {
            let mut m = BTreeMap::new();
            m.insert(format!("{}.{k}", labels::PREFIX), v.to_owned());
            m
        };
        let disabled = label("enable", "false");
        let scoped = label("scope", "media");
        let no_pull = label("no-pull", "true");
        let label_cases: Vec<(
            &BTreeMap<String, String>,
            LabelSettings<'_>,
            Exclusion,
            LabelRule,
        )> = vec![
            (
                &disabled,
                none(),
                Exclusion::LabelDisabled,
                LabelRule::Enable,
            ),
            (
                &empty,
                LabelSettings {
                    label_only: true,
                    scope: None,
                },
                Exclusion::LabelOnlyMode,
                LabelRule::Enable,
            ),
            (
                &scoped,
                LabelSettings {
                    label_only: false,
                    scope: Some("infra"),
                },
                Exclusion::ScopeMismatch {
                    scope: Some("media".into()),
                },
                LabelRule::Scope,
            ),
            (&no_pull, none(), Exclusion::NoPull, LabelRule::NoPull),
        ];
        for (l, s, want, rule) in label_cases {
            let p = effective(facts(Some("nginx"), l), auto, s);
            assert_eq!(p.excluded.as_ref(), Some(&want));
            assert_eq!(p.source, PolicySource::Label(rule));
        }
    }

    #[test]
    fn depths_itself_wins_over_labels_but_labels_are_still_shown() {
        let mut l = BTreeMap::new();
        l.insert(format!("{}.enable", labels::PREFIX), "false".to_owned());
        let p = effective(
            ServiceFacts {
                is_depths: true,
                ..facts(Some("depths:dev"), &l)
            },
            None,
            none(),
        );
        assert_eq!(p.excluded, Some(Exclusion::DepthsItself));
        assert_eq!(p.labels.len(), 1);
    }
}

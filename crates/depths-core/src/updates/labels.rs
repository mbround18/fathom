//! Watchtower label compatibility (contracts/watchtower-labels.md, FR-019–FR-022).
//!
//! Labels only ever restrict: exclusions first, in the contract's order, then `monitor-only`
//! caps the stack policy at `notify`. Every Watchtower label is reported with what Depths did.

use std::collections::BTreeMap;

use super::policy::{EffectivePolicy, Exclusion, PolicySource, UpdatePolicy, stack_source};
use super::reference::ImageRef;

/// Every Watchtower label key starts with this.
pub const PREFIX: &str = "com.centurylinklabs.watchtower";

const HOOK_REASON: &str = "lifecycle hooks run commands in the container; Depths only runs commands through the guarded shell";
const DEPENDS_REASON: &str = "order comes from the compose file";
const STOP_SIGNAL_REASON: &str = "compose's stop_signal applies";

/// Images that are Watchtower or one of its known forks, after normalisation.
pub const WATCHTOWER_IMAGES: [&str; 5] = [
    "docker.io/containrrr/watchtower",
    "ghcr.io/containrrr/watchtower",
    "docker.io/nickfedor/watchtower",
    "ghcr.io/nicholas-fedor/watchtower",
    "docker.io/beatkind/watchtower",
];

/// The host settings that labels are read against.
#[derive(Debug, Clone, Copy, Default)]
pub struct LabelSettings<'a> {
    /// Only services labelled `enable=true` (Watchtower's `--label-enable`).
    pub label_only: bool,
    /// Depths' scope (Watchtower's `--scope`); `none` matches unscoped containers.
    pub scope: Option<&'a str>,
}

/// A label that decided a result, named `label:<key>` as a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LabelRule {
    Enable,
    MonitorOnly,
    Scope,
    NoPull,
}

impl LabelRule {
    /// The key without the prefix.
    pub fn key(self) -> &'static str {
        match self {
            Self::Enable => "enable",
            Self::MonitorOnly => "monitor-only",
            Self::Scope => "scope",
            Self::NoPull => "no-pull",
        }
    }
}

/// What Depths did with a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelEffect {
    Applied,
    NotRun { reason: String },
    NotUsed { reason: String },
    Unrecognised,
}

/// One Watchtower label on a service, with its effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchtowerLabel {
    pub key: String,
    pub value: String,
    pub effect: LabelEffect,
}

/// Go's `strconv.ParseBool`. Anything else is absent.
pub fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Some(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

/// The labels, read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelReading {
    pub enable: Option<bool>,
    pub monitor_only: Option<bool>,
    pub scope: Option<String>,
    pub no_pull: Option<bool>,
    /// The bare `P=true` that marks a Watchtower container.
    pub marks_watchtower: bool,
    pub shown: Vec<WatchtowerLabel>,
}

fn is_hook(rest: &str) -> bool {
    let Some(hook) = rest.strip_prefix("lifecycle.") else {
        return false;
    };
    let hook = hook.strip_suffix("-timeout").unwrap_or(hook);
    matches!(
        hook,
        "pre-check" | "post-check" | "pre-update" | "post-update"
    )
}

/// Read every Watchtower label, in key order.
pub fn read(labels: &BTreeMap<String, String>) -> LabelReading {
    let mut out = LabelReading::default();
    for (key, value) in labels {
        let rest = if key == PREFIX {
            None
        } else if let Some(r) = key.strip_prefix(PREFIX).and_then(|r| r.strip_prefix('.')) {
            Some(r)
        } else {
            continue;
        };
        let bool_into = |slot: &mut Option<bool>| match parse_bool(value) {
            Some(b) => {
                *slot = Some(b);
                LabelEffect::Applied
            }
            None => LabelEffect::Unrecognised,
        };
        let effect = match rest {
            None => match parse_bool(value) {
                Some(b) => {
                    out.marks_watchtower = b;
                    LabelEffect::Applied
                }
                None => LabelEffect::Unrecognised,
            },
            Some("enable") => bool_into(&mut out.enable),
            Some("monitor-only") => bool_into(&mut out.monitor_only),
            Some("no-pull") => bool_into(&mut out.no_pull),
            Some("scope") => {
                out.scope = Some(value.clone());
                LabelEffect::Applied
            }
            Some(r) if is_hook(r) => LabelEffect::NotRun {
                reason: HOOK_REASON.into(),
            },
            Some("depends-on") => LabelEffect::NotRun {
                reason: DEPENDS_REASON.into(),
            },
            Some("stop-signal") => LabelEffect::NotUsed {
                reason: STOP_SIGNAL_REASON.into(),
            },
            Some(_) => LabelEffect::Unrecognised,
        };
        out.shown.push(WatchtowerLabel {
            key: key.clone(),
            value: value.clone(),
            effect,
        });
    }
    out
}

/// The label-driven exclusion, in the contract's order, with the rule that caused it.
fn exclusion(r: &LabelReading, settings: LabelSettings<'_>) -> Option<(Exclusion, LabelRule)> {
    if r.enable == Some(false) {
        return Some((Exclusion::LabelDisabled, LabelRule::Enable));
    }
    if settings.label_only && r.enable != Some(true) {
        return Some((Exclusion::LabelOnlyMode, LabelRule::Enable));
    }
    if let Some(want) = settings.scope {
        let matches = if want == "none" {
            matches!(r.scope.as_deref(), None | Some("none"))
        } else {
            r.scope.as_deref() == Some(want)
        };
        if !matches {
            return Some((
                Exclusion::ScopeMismatch {
                    scope: r.scope.clone(),
                },
                LabelRule::Scope,
            ));
        }
    }
    if r.no_pull == Some(true) {
        return Some((Exclusion::NoPull, LabelRule::NoPull));
    }
    None
}

/// The effective policy from the labels and the stack policy (`None`: no stored entry).
pub fn effective(
    labels: &BTreeMap<String, String>,
    stack_policy: Option<UpdatePolicy>,
    settings: LabelSettings<'_>,
) -> EffectivePolicy {
    let reading = read(labels);
    if let Some((excluded, rule)) = exclusion(&reading, settings) {
        return EffectivePolicy {
            policy: UpdatePolicy::Off,
            source: PolicySource::Label(rule),
            excluded: Some(excluded),
            labels: reading.shown,
        };
    }
    let stack = stack_policy.unwrap_or(UpdatePolicy::DEFAULT);
    let (policy, source) = if reading.monitor_only == Some(true) && stack > UpdatePolicy::Notify {
        (
            UpdatePolicy::Notify,
            PolicySource::Label(LabelRule::MonitorOnly),
        )
    } else {
        (stack, stack_source(stack_policy))
    };
    EffectivePolicy {
        policy,
        source,
        excluded: None,
        labels: reading.shown,
    }
}

/// A container is Watchtower when it runs and carries the bare marker label, or runs a known
/// Watchtower image.
pub fn is_watchtower(image: &str, labels: &BTreeMap<String, String>, running: bool) -> bool {
    if !running {
        return false;
    }
    if read(labels).marks_watchtower {
        return true;
    }
    ImageRef::parse(image).is_ok_and(|r| WATCHTOWER_IMAGES.contains(&r.name().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(k: &str) -> String {
        format!("{PREFIX}.{k}")
    }

    /// One row of the policy table: enable, monitor-only, scope, no-pull, label_only, Depths
    /// scope, stack policy.
    type Row = (
        u8,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        bool,
        Option<&'static str>,
        Option<UpdatePolicy>,
    );

    enum Want {
        X(Exclusion),
        P(UpdatePolicy, &'static str),
    }

    use UpdatePolicy::{Auto, Notify, Off};
    use Want::{P, X};

    fn rows() -> Vec<(Row, Want)> {
        let a = Some(Auto);
        let n = Some(Notify);
        let s = |x: &'static str| Some(x);
        vec![
            (
                (1, None, None, None, None, false, None, None),
                P(Notify, "default"),
            ),
            (
                (2, None, None, None, None, false, None, a),
                P(Auto, "stack_policy"),
            ),
            (
                (3, s("false"), None, None, None, false, None, a),
                X(Exclusion::LabelDisabled),
            ),
            (
                (4, s("False"), None, None, None, false, None, n),
                X(Exclusion::LabelDisabled),
            ),
            (
                (5, s("0"), None, None, None, false, None, a),
                X(Exclusion::LabelDisabled),
            ),
            (
                (6, s("true"), None, None, None, false, None, a),
                P(Auto, "stack_policy"),
            ),
            (
                (7, None, None, None, None, true, None, a),
                X(Exclusion::LabelOnlyMode),
            ),
            (
                (8, s("true"), None, None, None, true, None, a),
                P(Auto, "stack_policy"),
            ),
            (
                (9, s("false"), None, None, None, true, None, a),
                X(Exclusion::LabelDisabled),
            ),
            (
                (10, s("yes"), None, None, None, true, None, a),
                X(Exclusion::LabelOnlyMode),
            ),
            (
                (11, s("yes"), None, None, None, false, None, a),
                P(Auto, "stack_policy"),
            ),
            (
                (12, None, s("true"), None, None, false, None, a),
                P(Notify, "label:monitor-only"),
            ),
            (
                (13, None, s("true"), None, None, false, None, n),
                P(Notify, "stack_policy"),
            ),
            (
                (14, None, s("t"), None, None, false, None, a),
                P(Notify, "label:monitor-only"),
            ),
            (
                (15, None, s("false"), None, None, false, None, a),
                P(Auto, "stack_policy"),
            ),
            (
                (16, None, s("true"), None, None, false, None, Some(Off)),
                P(Off, "stack_policy"),
            ),
            (
                (17, None, None, s("media"), None, false, None, a),
                P(Auto, "stack_policy"),
            ),
            (
                (18, None, None, s("media"), None, false, s("media"), a),
                P(Auto, "stack_policy"),
            ),
            (
                (19, None, None, s("media"), None, false, s("infra"), a),
                X(Exclusion::ScopeMismatch {
                    scope: s("media").map(Into::into),
                }),
            ),
            (
                (20, None, None, None, None, false, s("media"), a),
                X(Exclusion::ScopeMismatch { scope: None }),
            ),
            (
                (21, None, None, None, None, false, s("none"), a),
                P(Auto, "stack_policy"),
            ),
            (
                (22, None, None, s("none"), None, false, s("none"), a),
                P(Auto, "stack_policy"),
            ),
            (
                (23, None, None, s("media"), None, false, s("none"), a),
                X(Exclusion::ScopeMismatch {
                    scope: s("media").map(Into::into),
                }),
            ),
            (
                (24, None, None, s("Media"), None, false, s("media"), a),
                X(Exclusion::ScopeMismatch {
                    scope: s("Media").map(Into::into),
                }),
            ),
            (
                (25, None, None, None, s("true"), false, None, a),
                X(Exclusion::NoPull),
            ),
            (
                (
                    26,
                    s("true"),
                    s("true"),
                    s("media"),
                    None,
                    true,
                    s("media"),
                    a,
                ),
                P(Notify, "label:monitor-only"),
            ),
            (
                (
                    27,
                    s("false"),
                    s("true"),
                    s("media"),
                    s("true"),
                    true,
                    s("infra"),
                    a,
                ),
                X(Exclusion::LabelDisabled),
            ),
            (
                (28, None, None, s("media"), s("true"), false, s("infra"), a),
                X(Exclusion::ScopeMismatch {
                    scope: s("media").map(Into::into),
                }),
            ),
        ]
    }

    #[test]
    fn vectors_1_to_28_policy_table() {
        for ((n, enable, monitor, scope, no_pull, label_only, depths_scope, stack), want) in rows()
        {
            let mut labels = BTreeMap::new();
            for (k, v) in [
                ("enable", enable),
                ("monitor-only", monitor),
                ("scope", scope),
                ("no-pull", no_pull),
            ] {
                if let Some(v) = v {
                    labels.insert(p(k), v.to_owned());
                }
            }
            let got = effective(
                &labels,
                stack,
                LabelSettings {
                    label_only,
                    scope: depths_scope,
                },
            );
            match want {
                X(reason) => {
                    assert_eq!(got.excluded, Some(reason), "vector {n}");
                    assert!(!got.allows_auto(), "vector {n}");
                }
                P(policy, source) => {
                    assert_eq!(got.excluded, None, "vector {n}");
                    assert_eq!(
                        (got.policy, got.source.to_string().as_str()),
                        (policy, source),
                        "vector {n}"
                    );
                }
            }
            // Every label given is shown.
            assert_eq!(got.labels.len(), labels.len(), "vector {n}");
        }
    }

    #[test]
    fn vectors_10_11_unrecognised_enable_is_shown() {
        let mut labels = BTreeMap::new();
        labels.insert(p("enable"), "yes".to_owned());
        for label_only in [true, false] {
            let got = effective(
                &labels,
                Some(Auto),
                LabelSettings {
                    label_only,
                    scope: None,
                },
            );
            assert_eq!(got.labels[0].effect, LabelEffect::Unrecognised);
        }
    }

    #[test]
    fn vector_17_scope_is_shown_without_a_depths_scope() {
        let mut labels = BTreeMap::new();
        labels.insert(p("scope"), "media".to_owned());
        let got = effective(&labels, Some(Auto), LabelSettings::default());
        assert_eq!(
            got.labels,
            vec![WatchtowerLabel {
                key: p("scope"),
                value: "media".into(),
                effect: LabelEffect::Applied
            }]
        );
    }

    fn shown(pairs: &[(&str, &str)]) -> Vec<WatchtowerLabel> {
        let labels: BTreeMap<String, String> =
            pairs.iter().map(|(k, v)| (p(k), (*v).to_owned())).collect();
        let got = effective(&labels, Some(Auto), LabelSettings::default());
        // Hook and order labels never change the policy.
        assert_eq!((got.policy, got.excluded), (Auto, None));
        got.labels
    }

    #[test]
    fn vector_29_lifecycle_hook_not_run() {
        let got = shown(&[("lifecycle.pre-update", "/backup.sh")]);
        assert_eq!(
            got[0].effect,
            LabelEffect::NotRun {
                reason: HOOK_REASON.into()
            }
        );
        assert_eq!(
            HOOK_REASON,
            "lifecycle hooks run commands in the container; Depths only runs commands through the guarded shell"
        );
    }

    #[test]
    fn vector_30_hook_and_its_timeout_not_run() {
        let got = shown(&[
            ("lifecycle.post-check", "/check.sh"),
            ("lifecycle.post-check-timeout", "5"),
        ]);
        assert_eq!(got.len(), 2);
        assert!(
            got.iter()
                .all(|l| matches!(l.effect, LabelEffect::NotRun { .. }))
        );
        for k in [
            "pre-check",
            "pre-update-timeout",
            "post-update",
            "post-update-timeout",
        ] {
            assert!(
                matches!(
                    shown(&[(&format!("lifecycle.{k}"), "x")])[0].effect,
                    LabelEffect::NotRun { .. }
                ),
                "{k}"
            );
        }
    }

    #[test]
    fn vector_31_depends_on_not_run() {
        let got = shown(&[("depends-on", "db,cache")]);
        assert_eq!(
            got[0].effect,
            LabelEffect::NotRun {
                reason: "order comes from the compose file".into()
            }
        );
    }

    #[test]
    fn vector_32_stop_signal_not_used() {
        let got = shown(&[("stop-signal", "SIGHUP")]);
        assert_eq!(
            got[0].effect,
            LabelEffect::NotUsed {
                reason: "compose's stop_signal applies".into()
            }
        );
    }

    #[test]
    fn vector_33_unknown_key_unrecognised() {
        assert_eq!(
            shown(&[("foo", "bar")])[0].effect,
            LabelEffect::Unrecognised
        );
        // Another prefix entirely is not a Watchtower label.
        let mut labels = BTreeMap::new();
        labels.insert("com.centurylinklabs.watchtowerx".to_owned(), "1".to_owned());
        assert!(read(&labels).shown.is_empty());
    }

    #[test]
    fn vectors_34_to_38_detection() {
        let none = BTreeMap::new();
        let mut marked = BTreeMap::new();
        marked.insert(PREFIX.to_owned(), "true".to_owned());
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let ghcr = format!("ghcr.io/containrrr/watchtower@{digest}");
        type Case<'a> = (u8, &'a str, &'a BTreeMap<String, String>, bool, bool);
        let cases: [Case<'_>; 5] = [
            (34, "containrrr/watchtower:latest", &none, true, true),
            (35, &ghcr, &none, true, true),
            (36, "myorg/watchtower-dashboard", &none, true, false),
            (37, "custom/updater", &marked, true, true),
            (38, "containrrr/watchtower", &none, false, false),
        ];
        for (n, image, labels, running, want) in cases {
            assert_eq!(is_watchtower(image, labels, running), want, "vector {n}");
        }
        for image in [
            "nickfedor/watchtower",
            "ghcr.io/nicholas-fedor/watchtower:1",
            "beatkind/watchtower",
        ] {
            assert!(is_watchtower(image, &none, true), "{image}");
        }
    }

    #[test]
    fn booleans_follow_go_parse_bool() {
        for t in ["1", "t", "T", "TRUE", "true", "True"] {
            assert_eq!(parse_bool(t), Some(true), "{t}");
        }
        for f in ["0", "f", "F", "FALSE", "false", "False"] {
            assert_eq!(parse_bool(f), Some(false), "{f}");
        }
        for other in ["yes", "on", " true", "", "tRUE", "no", "2"] {
            assert_eq!(parse_bool(other), None, "{other:?}");
        }
    }
}

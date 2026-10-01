//! 045 FR-031: project the already-validated `HostedConfig` into the issuer list as a seeded,
//! read-only issuer, so a hosted server whose own sign-in is OIDC does not render an empty list.
//!
//! The projection is pure and introduces no second source of truth: every field comes from the
//! configuration the process already validated before it bound a socket, and `client_secret` is
//! simply not among them (FR-033 -- no secret value is ever rendered or exported).
//!
//! This module also owns the gate on that surface (FR-034, FR-035). Who may review the
//! deployment's OIDC configuration is the cluster's decision, not Fathom's: the question is put
//! to the API server as a `SelfSubjectAccessReview` on the object the seed is mounted from,
//! performed as the signed-in user. Anything short of an explicit `allowed: true` is a refusal,
//! and an answer that cannot be read at all is an *unknown* that refuses too -- never a pass.

use fathom::protocol::{
    CapabilityResult, CapabilitySource, IssuerOrigin, NotificationLevel, OidcConfig,
    OidcIssuerConfig, SystemEvent,
};
use serde_json::json;

use crate::config::HostedConfig;

/// Id of the seeded issuer. Fixed, because there is exactly one and the UI keys its read-only
/// rendering on `OidcSeedOrigin.issuer_id` matching it.
pub const SEED_ISSUER_ID: &str = "hosted";

/// Default object the seed is mounted from, when the deployment does not name one.
const DEFAULT_SEED_OBJECT: &str = "secret/fathom-oidc";

/// The in-cluster file every pod gets, including a pass-through hosted pod: the chart mounts the
/// namespace through the downward API even where it mounts no ServiceAccount token.
const NAMESPACE_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// Project `cfg` into the issuer the hosted UI reviews (FR-031).
///
/// `issuer_url` carries `issuer_raw`, not the parsed `Url`: OIDC issuer matching is exact and
/// `Url` would append a `/` to a bare host, so the operator would be shown an issuer the
/// provider never asserts.
pub fn seeded_issuer(cfg: &HostedConfig) -> OidcIssuerConfig {
    OidcIssuerConfig {
        id: SEED_ISSUER_ID.to_owned(),
        issuer_url: cfg.oidc.issuer_raw.clone(),
        client_id: cfg.oidc.client_id.clone(),
        scopes: cfg.oidc.scopes.clone(),
        ca_bundle_path: cfg
            .oidc
            .ca_file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        username_claim: cfg.claims.username.clone(),
        groups_claim: cfg.claims.groups.clone(),
        // An unset prefix is *absent*, not the empty string: `HostedConfig` defaults both to ""
        // and the API server treats "" and "not set" differently for `--oidc-username-prefix`.
        username_prefix: non_empty(&cfg.claims.username_prefix),
        groups_prefix: non_empty(&cfg.claims.groups_prefix),
        // The read-only IdP directory is a desktop feature configured per issuer; a hosted
        // deployment has no admin key and this projection must not invent one. `client_secret`
        // is absent from this struct by design (FR-033): there is nothing to project it into.
        admin_api_base: None,
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// Which kind of object the seed is mounted from (FR-033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedObjectKind {
    Secret,
    ConfigMap,
}

impl SeedObjectKind {
    /// The plural name a `SelfSubjectAccessReview` asks about.
    pub fn resource(self) -> &'static str {
        match self {
            Self::Secret => "secrets",
            Self::ConfigMap => "configmaps",
        }
    }

    /// How the object is named to the operator.
    pub fn label(self) -> &'static str {
        match self {
            Self::Secret => "Secret",
            Self::ConfigMap => "ConfigMap",
        }
    }
}

/// The object the `FATHOM_OIDC_*` environment is mounted from, and therefore the object FR-034
/// reviews access against.
///
/// Read straight from the environment rather than carried on [`HostedConfig`], following
/// `host::probe_namespaces`' precedent for deployment-shaped knobs that need no validation
/// before the socket binds: naming the object wrongly costs a refused review, not a broken
/// sign-in, and a refused review is the fail-closed outcome FR-035 already requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedObjectRef {
    pub kind: SeedObjectKind,
    pub name: String,
    pub namespace: String,
}

impl SeedObjectRef {
    /// From the process environment, with the pod's own namespace as the default.
    pub fn from_env() -> Result<Self, String> {
        let own = std::fs::read_to_string(NAMESPACE_FILE).ok();
        Self::from_lookup(
            |k| std::env::var(k).ok().filter(|v| !v.is_empty()),
            own.as_deref(),
        )
    }

    /// From an arbitrary key lookup (testable). `Err` is the verbatim unknown to state.
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
        own_namespace: Option<&str>,
    ) -> Result<Self, String> {
        let spec = get("FATHOM_OIDC_CONFIG_OBJECT").unwrap_or_else(|| DEFAULT_SEED_OBJECT.into());
        let (kind, name) = match spec.trim().split_once('/') {
            Some(("secret" | "secrets", n)) => (SeedObjectKind::Secret, n),
            Some(("configmap" | "configmaps", n)) => (SeedObjectKind::ConfigMap, n),
            Some((other, _)) => {
                return Err(format!(
                    "FATHOM_OIDC_CONFIG_OBJECT names `{other}`: use secret/<name> or \
                     configmap/<name>"
                ));
            }
            // A bare name is the Secret form, which is how FR-033's seed is normally mounted.
            None => (SeedObjectKind::Secret, spec.trim()),
        };
        if name.is_empty() {
            return Err("FATHOM_OIDC_CONFIG_OBJECT names no object".to_owned());
        }
        let namespace = get("FATHOM_NAMESPACE")
            .or_else(|| own_namespace.map(|n| n.trim().to_owned()))
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                format!(
                    "cannot tell which namespace holds `{}`: no {NAMESPACE_FILE} and no \
                     FATHOM_NAMESPACE",
                    spec.trim()
                )
            })?;
        Ok(Self {
            kind,
            name: name.to_owned(),
            namespace,
        })
    }

    /// Where the operator changes the seed, in their words (FR-032).
    pub fn source(&self) -> String {
        format!(
            "FATHOM_OIDC_* (mounted {} {}/{})",
            self.kind.label(),
            self.namespace,
            self.name
        )
    }

    /// The review's subject, as the API server names it.
    fn verb_phrase(&self) -> String {
        format!(
            "get {} {}/{}",
            self.kind.resource(),
            self.namespace,
            self.name
        )
    }
}

/// The events that put the seeded issuer in the hosted issuer list (FR-031, FR-032).
///
/// `OidcConfigLoaded` comes first so the issuer exists by the time the origin that freezes it
/// arrives. `identities` is empty on purpose: the hosted session's identity already reaches the
/// UI as the `subscribed` frame's `PublicIdentity`, and a second representation of it here would
/// be exactly the duplicated source of truth FR-031 warns about.
pub fn seed_events(cfg: &HostedConfig, obj: &SeedObjectRef) -> Vec<SystemEvent> {
    vec![
        SystemEvent::OidcConfigLoaded {
            config: OidcConfig {
                issuers: vec![seeded_issuer(cfg)],
                contexts: Vec::new(),
                role_map: Default::default(),
            },
            identities: Vec::new(),
        },
        SystemEvent::OidcSeedOrigin {
            issuer_id: SEED_ISSUER_ID.to_owned(),
            origin: IssuerOrigin::SeededFromEnvironment,
            source: obj.source(),
        },
    ]
}

/// Ask the API server, as the signed-in user, whether they may read the object the OIDC
/// configuration is mounted from (FR-034).
pub async fn review_config_access(client: kube::Client, obj: &SeedObjectRef) -> CapabilityResult {
    let body = json!({
        "apiVersion": "authorization.k8s.io/v1",
        "kind": "SelfSubjectAccessReview",
        "spec": { "resourceAttributes": {
            "group": "",
            "resource": obj.kind.resource(),
            "namespace": obj.namespace,
            "name": obj.name,
            "verb": "get",
        }},
    });
    // Built by hand rather than through `Api<SelfSubjectAccessReview>`: the server crate does
    // not depend on k8s-openapi, and a review carries no name for a typed `create` to set.
    let answer = serde_json::to_vec(&body)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            http::Request::builder()
                .method("POST")
                .uri("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews")
                .header("content-type", "application/json")
                .body(bytes)
                .map_err(|e| e.to_string())
        });
    let answer = match answer {
        Ok(req) => client
            .request::<serde_json::Value>(req)
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    decide(answer, obj)
}

/// The pure half of FR-034/FR-035: turn whatever the API server said into the verdict.
///
/// Three outcomes, never two: allowed, definitely denied, and *undetermined* -- the same shape
/// the API server read of FR-010 uses. An error, or a body with no readable `status.allowed`,
/// is undetermined and refuses; it never falls through to allowed.
pub fn decide(answer: Result<serde_json::Value, String>, obj: &SeedObjectRef) -> CapabilityResult {
    let unknown = |reason: String| CapabilityResult {
        allowed: false,
        reason: Some(reason),
        source: CapabilitySource::Review,
        verified: false,
    };
    let body = match answer {
        Ok(body) => body,
        Err(e) => return unknown(e),
    };
    let status = match body.get("status") {
        Some(s) if s.is_object() => s,
        // A 200 whose body is not a review is not an answer. Printing it verbatim is what lets
        // the operator see *why* (a proxy's HTML error page, an empty body, a changed shape).
        _ => return unknown(format!("the review returned no status: {body}")),
    };
    let reason = status
        .get("reason")
        .and_then(|v| v.as_str())
        .or_else(|| status.get("evaluationError").and_then(|v| v.as_str()))
        .map(str::to_owned);
    match status.get("allowed").and_then(serde_json::Value::as_bool) {
        Some(true) => CapabilityResult {
            allowed: true,
            reason,
            source: CapabilitySource::Review,
            verified: true,
        },
        Some(false) => CapabilityResult {
            allowed: false,
            reason: Some(reason.unwrap_or_else(|| {
                format!(
                    "the API server refused `{}` without a reason",
                    obj.verb_phrase()
                )
            })),
            source: CapabilitySource::Review,
            verified: true,
        },
        None => unknown(format!(
            "the review did not say whether `{}` is allowed: {status}",
            obj.verb_phrase()
        )),
    }
}

/// What the operator is told when the review of FR-034 did not allow the surface. The API
/// server's own words, verbatim, with the question it answered named.
pub fn denied_notice(result: &CapabilityResult, obj: &SeedObjectRef) -> SystemEvent {
    let reason = result.reason.as_deref().unwrap_or("no reason given");
    let message = if result.verified {
        format!(
            "Fathom's OIDC configuration is not shown: the API server refused `{}` — {reason}",
            obj.verb_phrase()
        )
    } else {
        format!(
            "Fathom's OIDC configuration is not shown: `{}` could not be checked, so it is \
             treated as not permitted — {reason}",
            obj.verb_phrase()
        )
    };
    SystemEvent::Notification {
        level: NotificationLevel::Warn,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&'static str, &str)]) -> HashMap<&'static str, String> {
        pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
    }

    fn config(extra: &[(&'static str, &str)]) -> HostedConfig {
        let mut m = env(&[
            ("FATHOM_EXTERNAL_URL", "https://fathom.example.com"),
            ("FATHOM_OIDC_ISSUER", "https://idp.example.com/realms/x"),
            ("FATHOM_OIDC_CLIENT_ID", "fathom"),
        ]);
        m.extend(env(extra));
        HostedConfig::from_lookup(|k| m.get(k).cloned(), false).expect("valid config")
    }

    fn obj() -> SeedObjectRef {
        SeedObjectRef {
            kind: SeedObjectKind::Secret,
            name: "fathom-oidc".to_owned(),
            namespace: "fathom".to_owned(),
        }
    }

    /// T065/FR-031: the projection carries what the server validated, field for field.
    #[test]
    fn projection_carries_the_validated_configuration() {
        let cfg = config(&[
            ("FATHOM_OIDC_SCOPES", "openid,groups"),
            ("FATHOM_CLAIM_USERNAME", "email"),
            ("FATHOM_CLAIM_GROUPS", "roles"),
            ("FATHOM_USERNAME_PREFIX", "oidc:"),
            ("FATHOM_GROUPS_PREFIX", "oidc:"),
            ("FATHOM_OIDC_CA_FILE", "/etc/fathom/ca/ca.crt"),
        ]);
        let issuer = seeded_issuer(&cfg);
        assert_eq!(issuer.id, SEED_ISSUER_ID);
        assert_eq!(issuer.client_id, "fathom");
        assert_eq!(issuer.scopes, ["openid", "groups"]);
        assert_eq!(issuer.username_claim, "email");
        assert_eq!(issuer.groups_claim, "roles");
        assert_eq!(issuer.username_prefix.as_deref(), Some("oidc:"));
        assert_eq!(issuer.groups_prefix.as_deref(), Some("oidc:"));
        assert_eq!(
            issuer.ca_bundle_path.as_deref(),
            Some("/etc/fathom/ca/ca.crt")
        );
        assert_eq!(issuer.admin_api_base, None);
    }

    /// The issuer string is matched exactly by every provider: `Url` would append a `/` to a
    /// bare host and the operator would be shown an issuer that no token ever carries.
    #[test]
    fn projection_keeps_the_issuer_verbatim() {
        for raw in [
            "https://idp.example.com/auth/v1",
            "https://idp.example.com/auth/v1/",
            "https://idp.example.com",
        ] {
            let cfg = config(&[("FATHOM_OIDC_ISSUER", raw)]);
            assert_eq!(seeded_issuer(&cfg).issuer_url, raw);
            // The parsed `Url` is the thing this must not be.
            assert_ne!(
                seeded_issuer(&cfg).issuer_url,
                "https://idp.example.com/".to_owned(),
                "{raw} was normalized"
            );
        }
    }

    /// An unset prefix is absent, not empty: the API server distinguishes the two.
    #[test]
    fn unset_prefixes_project_as_absent() {
        let issuer = seeded_issuer(&config(&[]));
        assert_eq!(issuer.username_prefix, None);
        assert_eq!(issuer.groups_prefix, None);
        assert_eq!(issuer.ca_bundle_path, None);
    }

    /// T065/FR-033: the client secret is never projected, so it cannot be rendered or exported.
    #[test]
    fn the_client_secret_is_never_projected() {
        const SECRET: &str = "s3cr3t-client-secret-value";
        let cfg = config(&[("FATHOM_OIDC_CLIENT_SECRET", SECRET)]);
        let wire = serde_json::to_string(&seeded_issuer(&cfg)).expect("serializable");
        assert!(
            !wire.contains(SECRET),
            "the client secret reached the projection: {wire}"
        );
        // And the whole surfaced payload, not just the issuer struct.
        let events = serde_json::to_string(&seed_events(&cfg, &obj())).expect("serializable");
        assert!(
            !events.contains(SECRET),
            "the client secret reached the wire"
        );
        assert!(events.contains("SeededFromEnvironment"));
        assert!(events.contains("mounted Secret fathom/fathom-oidc"));
    }

    #[test]
    fn the_seed_object_defaults_to_the_mounted_secret_in_the_pods_namespace() {
        let r = SeedObjectRef::from_lookup(|_| None, Some("fathom\n")).expect("resolved");
        assert_eq!(r.kind, SeedObjectKind::Secret);
        assert_eq!(r.name, "fathom-oidc");
        assert_eq!(r.namespace, "fathom");
        assert_eq!(
            r.source(),
            "FATHOM_OIDC_* (mounted Secret fathom/fathom-oidc)"
        );
    }

    #[test]
    fn the_seed_object_can_be_a_configmap_and_an_explicit_namespace() {
        let m = env(&[
            ("FATHOM_OIDC_CONFIG_OBJECT", "configmap/fathom-oidc-seed"),
            ("FATHOM_NAMESPACE", "platform"),
        ]);
        let r = SeedObjectRef::from_lookup(|k| m.get(k).cloned(), None).expect("resolved");
        assert_eq!(r.kind, SeedObjectKind::ConfigMap);
        assert_eq!(r.name, "fathom-oidc-seed");
        assert_eq!(r.namespace, "platform");
    }

    /// An object that cannot be resolved is a stated unknown, not a guess.
    #[test]
    fn an_unresolvable_seed_object_is_an_error_with_its_reason() {
        let m = env(&[("FATHOM_OIDC_CONFIG_OBJECT", "deployment/fathom")]);
        let e = SeedObjectRef::from_lookup(|k| m.get(k).cloned(), Some("fathom")).unwrap_err();
        assert!(e.contains("deployment"), "{e}");
        let e = SeedObjectRef::from_lookup(|_| None, None).unwrap_err();
        assert!(e.contains("FATHOM_NAMESPACE"), "{e}");
    }

    /// T070/FR-034: an allowed review is the only thing that opens the surface, and it is the
    /// API server that decided -- no Fathom-internal group list is consulted anywhere here.
    #[test]
    fn an_allowed_review_opens_the_surface() {
        let r = decide(Ok(json!({ "status": { "allowed": true } })), &obj());
        assert!(r.allowed && r.verified);
        assert_eq!(r.source, CapabilitySource::Review);
    }

    /// T070/FR-034: a denial carries the API server's own words.
    #[test]
    fn a_denied_review_is_refused_in_the_api_servers_words() {
        let reason = "secrets \"fathom-oidc\" is forbidden: User \"oidc:carol\" cannot get \
                      resource \"secrets\" in API group \"\" in the namespace \"fathom\"";
        let r = decide(
            Ok(json!({ "status": { "allowed": false, "reason": reason } })),
            &obj(),
        );
        assert!(!r.allowed);
        assert!(r.verified, "a definite no is a completed review");
        assert_eq!(r.reason.as_deref(), Some(reason));
        let SystemEvent::Notification { level, message } = denied_notice(&r, &obj()) else {
            panic!("expected a notification");
        };
        assert_eq!(level, NotificationLevel::Warn);
        assert!(message.contains(reason), "{message}");
        assert!(
            message.contains("get secrets fathom/fathom-oidc"),
            "{message}"
        );
    }

    /// T070/FR-035: fail closed. Every answer that is not a readable `allowed` is an unknown
    /// that refuses, with the unknown stated verbatim -- never an assumed pass.
    #[test]
    fn an_unreadable_review_fails_closed_with_the_unknown_stated() {
        let cases: Vec<(Result<serde_json::Value, String>, &str)> = vec![
            (
                Err("error trying to connect: tcp connect error".into()),
                "tcp connect error",
            ),
            (Ok(json!({})), "no status"),
            (Ok(json!({ "status": {} })), "did not say whether"),
            (
                Ok(json!({ "status": { "allowed": "yes" } })),
                "did not say whether",
            ),
            (Ok(json!({ "status": null })), "no status"),
            (
                Ok(json!({ "status": { "evaluationError": "webhook unavailable" } })),
                "did not say whether",
            ),
        ];
        for (answer, needle) in cases {
            let r = decide(answer.clone(), &obj());
            assert!(!r.allowed, "{answer:?} fell through to allowed");
            assert!(!r.verified, "{answer:?} claimed a completed review");
            let reason = r.reason.clone().unwrap_or_default();
            assert!(reason.contains(needle), "{answer:?} -> {reason}");
            let SystemEvent::Notification { message, .. } = denied_notice(&r, &obj()) else {
                panic!("expected a notification");
            };
            assert!(message.contains(&reason), "{message}");
            assert!(message.contains("not permitted"), "{message}");
        }
    }

    /// An `allowed: true` answer that also carries an evaluation error stays allowed: the API
    /// server made the call, and this module does not second-guess a yes.
    #[test]
    fn a_yes_with_an_evaluation_error_is_still_a_yes() {
        let r = decide(
            Ok(json!({ "status": { "allowed": true, "evaluationError": "partial" } })),
            &obj(),
        );
        assert!(r.allowed && r.verified);
        assert_eq!(r.reason.as_deref(), Some("partial"));
    }
}

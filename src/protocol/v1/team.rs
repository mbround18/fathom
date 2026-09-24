//! Teams & Organizations (self-hosted), spec `014-teams-organizations`.
//!
//! Two independent, additive layers:
//! - **Connection profiles** (US1): shareable, non-secret cluster-connection metadata
//!   (API server endpoint, context alias, default namespace). Never credentials.
//! - **Cluster Settings + org sync** (US2): the existing §10 field set (metrics source,
//!   Prometheus path, HTTPS toggle, path prefix, hidden metrics), local per-operator,
//!   optionally published as an org-level default that teammates must explicitly accept.
//!
//! The shared config store is a self-hosted, team-controlled directory (a local path, or
//! a mounted network share) that the operator's install points at — never a Fathom-hosted
//! service (FR-015). Persistence: `src/backend/team_store.rs`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------------------------------------
// Cluster Settings (§10): local, per-operator, per-context.
// ---------------------------------------------------------------------------------------

pub const CLUSTER_SETTINGS_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum MetricsSource {
    #[default]
    None,
    Prometheus,
}

/// Per-field provenance: is this field following the org-level default, or has the
/// operator locally overridden it? Org sync (FR-007/FR-008) must never clobber a
/// `Local` field without explicit confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SettingOrigin {
    #[default]
    Local,
    OrgDefault,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSettingsFieldOrigin {
    #[serde(default)]
    pub metrics_source: SettingOrigin,
    #[serde(default)]
    pub prometheus_path: SettingOrigin,
    #[serde(default)]
    pub https: SettingOrigin,
    #[serde(default)]
    pub path_prefix: SettingOrigin,
    #[serde(default)]
    pub hidden_metrics: SettingOrigin,
}

/// `~/.fathom/cluster-settings.json`, keyed by kubeconfig context name. §10's field set,
/// exactly — this feature MUST NOT add any other preference category here (FR-006).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSettings {
    #[ts(type = "1")]
    pub schema: u32,
    #[serde(default)]
    pub metrics_source: MetricsSource,
    #[serde(default)]
    pub prometheus_path: String,
    #[serde(default)]
    pub https: bool,
    #[serde(default)]
    pub path_prefix: String,
    #[serde(default)]
    pub hidden_metrics: Vec<String>,
    /// Set when this context is a shared team connection; org-sync affordances only
    /// appear when `Some` (FR-010).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_rev_known: Option<u64>,
    #[serde(default)]
    pub field_origin: ClusterSettingsFieldOrigin,
}

impl Default for ClusterSettings {
    fn default() -> Self {
        Self {
            schema: CLUSTER_SETTINGS_SCHEMA,
            metrics_source: MetricsSource::default(),
            prometheus_path: "/metrics".to_owned(),
            https: false,
            path_prefix: String::new(),
            hidden_metrics: Vec::new(),
            org_rev_known: None,
            field_origin: ClusterSettingsFieldOrigin::default(),
        }
    }
}

impl ClusterSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CLUSTER_SETTINGS_SCHEMA {
            return Err(format!(
                "unsupported clusterSettings.schema {} (expected {CLUSTER_SETTINGS_SCHEMA})",
                self.schema
            ));
        }
        Ok(())
    }
}

/// Org-level default for one shared cluster connection, as stored in the shared config
/// store. Same field set as `ClusterSettings` minus provenance/local-only bookkeeping,
/// plus publication metadata used for conflict detection (FR-012).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrgClusterSettings {
    pub context: String,
    /// Monotonically increasing; a publish must supply the `rev` it last saw or be
    /// refused with a conflict (optimistic concurrency).
    pub rev: u64,
    pub published_by: String,
    pub published_at: String,
    pub metrics_source: MetricsSource,
    pub prometheus_path: String,
    pub https: bool,
    pub path_prefix: String,
    pub hidden_metrics: Vec<String>,
}

// ---------------------------------------------------------------------------------------
// Connection profiles (US1): non-secret only.
// ---------------------------------------------------------------------------------------

/// Shareable, non-secret metadata describing how to reach one cluster context. This type
/// intentionally has no field capable of carrying a credential, token, or kubeconfig auth
/// material — nothing more than these four fields can ever be synced (FR-001/FR-002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    pub context_alias: String,
    pub api_server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_namespace: Option<String>,
    pub rev: u64,
    pub shared_by: String,
    pub updated_at: String,
    /// `true` once removed upstream. Kept (not deleted) so an operator who already
    /// adopted it can be told "no longer shared" instead of losing the row (FR-004).
    #[serde(default)]
    pub removed: bool,
}

/// Local record of a connection profile the operator has adopted: the last-known shared
/// fields plus local adoption state. Removing team-store access, or the upstream profile
/// going away, MUST NOT delete this — it just stops following the shared copy
/// (Constitution Principle VI / edge case in spec.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AdoptedConnectionProfile {
    pub profile: ConnectionProfile,
    pub adopted_at: String,
    /// `false` once the upstream profile is confirmed gone from the shared store.
    #[serde(default = "default_true")]
    pub still_shared: bool,
}

fn default_true() -> bool {
    true
}

/// Reachability/status of the configured shared config store (FR-005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamStoreStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_path: Option<String>,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A publish (connection profile or org settings) was refused because at least one
/// field looked like it contained credential/secret material, or because of a
/// rev conflict with a concurrent edit (FR-002, FR-012).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TeamPublishRejection {
    SuspectedSecret {
        field: String,
        reason: String,
    },
    Conflict {
        expected_rev: u64,
        current_rev: u64,
    },
    StoreUnreachable {
        reason: String,
    },
    /// 033 US3: the signed-in identity's Fathom role (viewers/editors/admins group) is too low.
    RoleForbidden {
        reason: String,
    },
}

// ---------------------------------------------------------------------------------------
// Non-secret enforcement (FR-002, SC-002).
// ---------------------------------------------------------------------------------------

/// Case-insensitive substrings that never belong in a connection-profile or cluster
/// context alias/endpoint/namespace field. Defense-in-depth: the *primary* guarantee is
/// type-level (`ConnectionProfile` simply has no field that can hold auth material), this
/// heuristic catches an operator pasting a credential into a free-text field by mistake.
const SECRET_MARKERS: &[&str] = &[
    "bearer ",
    "-----begin",
    "token=",
    "token:",
    "password=",
    "passwd=",
    "secret=",
    "apikey",
    "api_key",
    "api-key",
    "client_secret",
    "authorization:",
    "x-api-key",
    "ssh-rsa",
    "ssh-ed25519",
];

/// Flags a value that looks like a bare high-entropy token (JWT, base64 cert/key blob,
/// long hex secret) rather than a hostname, alias, or namespace.
fn looks_like_bare_token(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 40 || trimmed.contains(' ') {
        return false;
    }
    // JWTs and most token/cert blobs are made up of base64url-ish characters and don't
    // contain the punctuation a URL or DNS-ish alias would (`://`, and at most one `.`
    // run per label). A URL like `https://cluster.example.com:6443` still contains `://`
    // and multiple short dot-separated labels, so it's excluded here.
    if trimmed.contains("://") {
        return false;
    }
    // `.` is allowed here (JWTs are three dot-separated base64url segments); a real URL
    // was already excluded above via the `://` check.
    let allowed =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/' | '=' | '.');
    if !trimmed.chars().all(allowed) {
        return false;
    }
    let has_letters = trimmed.chars().any(|c| c.is_ascii_alphabetic());
    let has_digits = trimmed.chars().any(|c| c.is_ascii_digit());
    has_letters && has_digits
}

/// Returns `Err((field, reason))` for the first field that looks like it carries
/// credential/secret material. Used to gate every publish of a connection profile or
/// org-level cluster settings string field before it ever reaches the shared store.
pub fn scan_fields_for_secrets(fields: &[(&str, &str)]) -> Result<(), (String, String)> {
    for (name, value) in fields {
        let lower = value.to_lowercase();
        if let Some(marker) = SECRET_MARKERS.iter().find(|m| lower.contains(*m)) {
            return Err((
                (*name).to_owned(),
                format!("looks like it contains credential/secret material (matched \"{marker}\")"),
            ));
        }
        if looks_like_bare_token(value) {
            return Err((
                (*name).to_owned(),
                "looks like a bare token/credential rather than a hostname or alias".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_settings_round_trip() {
        let s = ClusterSettings {
            schema: CLUSTER_SETTINGS_SCHEMA,
            metrics_source: MetricsSource::Prometheus,
            prometheus_path: "/metrics".into(),
            https: true,
            path_prefix: "/proxy".into(),
            hidden_metrics: vec!["cpu_throttled".into()],
            org_rev_known: Some(3),
            field_origin: ClusterSettingsFieldOrigin {
                metrics_source: SettingOrigin::OrgDefault,
                prometheus_path: SettingOrigin::Local,
                https: SettingOrigin::OrgDefault,
                path_prefix: SettingOrigin::OrgDefault,
                hidden_metrics: SettingOrigin::Local,
            },
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ClusterSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        s.validate().unwrap();
    }

    #[test]
    fn cluster_settings_default_local_metrics_off() {
        let s = ClusterSettings::default();
        assert_eq!(s.metrics_source, MetricsSource::None);
        assert!(!s.https);
        assert!(s.org_rev_known.is_none());
    }

    #[test]
    fn org_cluster_settings_round_trip() {
        let o = OrgClusterSettings {
            context: "prod".into(),
            rev: 7,
            published_by: "alice".into(),
            published_at: "2026-09-23T00:00:00Z".into(),
            metrics_source: MetricsSource::Prometheus,
            prometheus_path: "/metrics".into(),
            https: true,
            path_prefix: String::new(),
            hidden_metrics: vec![],
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: OrgClusterSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(o, back);
    }

    #[test]
    fn connection_profile_round_trip() {
        let p = ConnectionProfile {
            id: "abc123".into(),
            context_alias: "prod-cluster".into(),
            api_server: "https://cluster.example.com:6443".into(),
            default_namespace: Some("default".into()),
            rev: 1,
            shared_by: "alice".into(),
            updated_at: "2026-09-23T00:00:00Z".into(),
            removed: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: ConnectionProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    /// Security property (SC-002): `ConnectionProfile`'s wire shape has exactly the
    /// non-secret fields the spec allows — nothing else can ever be serialized into a
    /// published profile, no matter what a future change adds elsewhere, unless it's
    /// added to this exact allow-list.
    #[test]
    fn connection_profile_wire_shape_has_no_secret_fields() {
        let p = ConnectionProfile {
            id: "abc123".into(),
            context_alias: "prod-cluster".into(),
            api_server: "https://cluster.example.com:6443".into(),
            default_namespace: Some("default".into()),
            rev: 1,
            shared_by: "alice".into(),
            updated_at: "2026-09-23T00:00:00Z".into(),
            removed: false,
        };
        let value = serde_json::to_value(&p).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "apiServer",
                "contextAlias",
                "defaultNamespace",
                "id",
                "removed",
                "rev",
                "sharedBy",
                "updatedAt",
            ]
        );
    }

    /// A credential/token pasted into a connection-profile field is refused before it
    /// ever reaches the shared store (FR-002 / SC-002): the single security-critical
    /// test the spec calls out explicitly.
    #[test]
    fn scan_fields_for_secrets_rejects_bearer_token() {
        let err = scan_fields_for_secrets(&[
            ("contextAlias", "prod"),
            (
                "apiServer",
                "https://cluster.example.com:6443 Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0",
            ),
        ]);
        assert!(err.is_err(), "expected a bearer token to be rejected");
    }

    #[test]
    fn scan_fields_for_secrets_rejects_bare_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let err = scan_fields_for_secrets(&[("defaultNamespace", jwt)]);
        assert!(err.is_err(), "expected a bare JWT to be rejected");
    }

    #[test]
    fn scan_fields_for_secrets_rejects_pem_block() {
        let err = scan_fields_for_secrets(&[(
            "apiServer",
            "-----BEGIN CERTIFICATE-----\nMIIC...\n-----END CERTIFICATE-----",
        )]);
        assert!(err.is_err(), "expected a PEM block to be rejected");
    }

    #[test]
    fn scan_fields_for_secrets_allows_ordinary_url_and_alias() {
        let ok = scan_fields_for_secrets(&[
            ("contextAlias", "prod-cluster"),
            ("apiServer", "https://cluster.example.com:6443"),
            ("defaultNamespace", "default"),
        ]);
        assert!(
            ok.is_ok(),
            "ordinary connection metadata must be allowed: {ok:?}"
        );
    }
}

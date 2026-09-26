//! Payload types for manifest and kubeconfig export (spec 040).
//!
//! Manifest export is read-only: the backend fetches objects fresh and the UI cleans and
//! renders them. Kubeconfig export keeps every credential value inside the backend: the UI
//! only ever sees a redacted preview and the result of the write.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Upper bound on objects in one `FetchManifests` request (FR-016).
pub const MANIFEST_EXPORT_MAX_OBJECTS: usize = 500;

/// One object to export, addressed by its API identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ManifestObjectRef {
    /// e.g. `apps/v1`, `v1`, `example.com/v1alpha1`.
    pub api_version: String,
    pub kind: String,
    /// `None` for cluster-scoped objects.
    pub namespace: Option<String>,
    pub name: String,
}

/// An object read for export. `raw_json` is what the API server returned, except that
/// Secret values are redacted and the last-applied annotation dropped (FR-011).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FetchedManifest {
    /// Position in the request's `objects`.
    pub index: u32,
    pub raw_json: String,
}

/// An object that could not be read, with the cluster's (or discovery's) error text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ManifestFetchError {
    pub index: u32,
    pub message: String,
}

/// How a kubeconfig user authenticates (FR-023). Never carries a credential value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum KubeconfigCredentialKind {
    /// `users[].user.exec`: exported verbatim; the plugin runs on the target machine.
    Exec {
        command: String,
    },
    /// Legacy `users[].user.auth-provider`.
    AuthProvider {
        name: String,
    },
    Token,
    TokenFile,
    ClientCertificate,
    Basic,
    None,
}

/// Redacted preview of a single-context kubeconfig export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KubeconfigExportPreview {
    pub context: String,
    /// The standalone kubeconfig with every secret field replaced by `<redacted>`.
    pub redacted_yaml: String,
    pub credential: KubeconfigCredentialKind,
    /// Secret fields present in the user entry, e.g. `token`, `auth-provider.config.id-token`.
    pub embedded_secret_fields: Vec<String>,
    /// File paths kept as-is that will not resolve on another machine.
    pub file_references: Vec<String>,
    pub warnings: Vec<String>,
    /// Default target, `~` already expanded.
    pub suggested_path: String,
    /// The kubeconfig file the context was read from (display only).
    pub source_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_kind_roundtrip() {
        for kind in [
            KubeconfigCredentialKind::Exec {
                command: "aws".into(),
            },
            KubeconfigCredentialKind::AuthProvider {
                name: "oidc".into(),
            },
            KubeconfigCredentialKind::Token,
            KubeconfigCredentialKind::TokenFile,
            KubeconfigCredentialKind::ClientCertificate,
            KubeconfigCredentialKind::Basic,
            KubeconfigCredentialKind::None,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(
                serde_json::from_str::<KubeconfigCredentialKind>(&json).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn object_ref_roundtrip() {
        let r = ManifestObjectRef {
            api_version: "apps/v1".into(),
            kind: "Deployment".into(),
            namespace: None,
            name: "web".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ManifestObjectRef>(&json).unwrap(), r);
    }
}

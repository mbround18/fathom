use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SecretSummary {
    pub name: String,
    pub namespace: String,
    pub secret_type: String,
    pub keys_count: usize,
}

/// SECURITY: This struct must never contain decoded/raw secret values.
/// Only key names and the secret type are exposed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SecretDetails {
    pub namespace: String,
    pub name: String,
    pub creation_timestamp: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub secret_type: String,
    pub keys: Vec<String>,
    pub raw_json: String,
}

/// Reply to `RevealSecretValue` only: one Secret key's value, fetched on demand when the
/// operator explicitly clicks Reveal. Never sent unprompted, never part of a list or of
/// `SecretDetails`, never logged (`Debug` redacts the value). The UI hands it to the waiting
/// Reveal button and does not keep it in the store.
///
/// SECURITY: this is the only protocol type that may carry a decoded secret value.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct SecretValueReveal {
    pub namespace: String,
    pub name: String,
    pub key: String,
    pub outcome: SecretRevealOutcome,
}

/// What a `RevealSecretValue` found. Failures the operator can act on are typed, so the UI
/// can say "forbidden" or "not found" plainly instead of showing a raw API error.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum SecretRevealOutcome {
    /// The key's value, decoded from base64 as UTF-8 text (may be empty).
    Value { value: String },
    /// The value is not valid UTF-8 (a keystore, a binary blob): not shown as text.
    Binary { size: usize },
    /// The Secret exists but has no such key.
    KeyMissing,
    /// The Secret no longer exists (404).
    NotFound,
    /// RBAC does not allow the current identity to `get` this Secret (403). `message` is the
    /// API server's reason.
    Forbidden { message: String },
    /// Any other failure (network, API error), verbatim.
    Failed { message: String },
}

impl std::fmt::Debug for SecretValueReveal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretValueReveal")
            .field("namespace", &self.namespace)
            .field("name", &self.name)
            .field("key", &self.key)
            .field("outcome", &self.outcome)
            .finish()
    }
}

impl std::fmt::Debug for SecretRevealOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Value { .. } => f.write_str("Value { value: <redacted> }"),
            Self::Binary { size } => write!(f, "Binary {{ size: {size} }}"),
            Self::KeyMissing => f.write_str("KeyMissing"),
            Self::NotFound => f.write_str("NotFound"),
            Self::Forbidden { message } => write!(f, "Forbidden {{ message: {message:?} }}"),
            Self::Failed { message } => write!(f, "Failed {{ message: {message:?} }}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_debug_never_shows_the_value() {
        let reveal = SecretValueReveal {
            namespace: "ns".into(),
            name: "db".into(),
            key: "password".into(),
            outcome: SecretRevealOutcome::Value {
                value: "hunter2".into(),
            },
        };
        let dbg = format!("{reveal:?}");
        assert!(!dbg.contains("hunter2"), "{dbg}");
        assert!(dbg.contains("password"), "{dbg}");
    }

    #[test]
    fn reveal_outcome_wire_shape() {
        let json = serde_json::to_string(&SecretRevealOutcome::Forbidden {
            message: "no".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"Forbidden","data":{"message":"no"}}"#);
        let json = serde_json::to_string(&SecretRevealOutcome::NotFound).unwrap();
        assert_eq!(json, r#"{"type":"NotFound"}"#);
    }
}

//! 037-node-shell: types for the privileged node debug pod and its attached terminal.
//!
//! Sessions live in backend memory only; only [`NodeShellSettings`] is persisted (per context,
//! additive, see `persistence.rs`). Every string that came from the API server or a pod's status
//! is carried verbatim (Constitution I).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const NODE_SHELL_DEFAULT_IMAGE: &str = "busybox:stable";
pub const NODE_SHELL_DEFAULT_NAMESPACE: &str = "default";
pub const NODE_SHELL_DEFAULT_DEADLINE_SECONDS: u32 = 14_400;
pub const NODE_SHELL_MIN_DEADLINE_SECONDS: u32 = 300;
pub const NODE_SHELL_MAX_DEADLINE_SECONDS: u32 = 86_400;

/// Per-context Node Shell settings (data-model.md). Missing keys fall back to their default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct NodeShellSettings {
    pub image: String,
    pub namespace: String,
    /// Secret *name* only; its contents are never read.
    pub image_pull_secret: Option<String>,
    pub deadline_seconds: u32,
}

impl Default for NodeShellSettings {
    fn default() -> Self {
        Self {
            image: NODE_SHELL_DEFAULT_IMAGE.to_owned(),
            namespace: NODE_SHELL_DEFAULT_NAMESPACE.to_owned(),
            image_pull_secret: None,
            deadline_seconds: NODE_SHELL_DEFAULT_DEADLINE_SECONDS,
        }
    }
}

impl NodeShellSettings {
    /// Trim, clamp the deadline, and reject an empty image or a non-DNS-label namespace.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.image = self.image.trim().to_owned();
        self.namespace = self.namespace.trim().to_owned();
        self.image_pull_secret = self
            .image_pull_secret
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if self.image.is_empty() {
            return Err("Node shell image must not be empty".to_owned());
        }
        if !is_dns_label(&self.namespace) {
            return Err(format!(
                "Node shell namespace '{}' is not a valid DNS label",
                self.namespace
            ));
        }
        self.deadline_seconds = self.deadline_seconds.clamp(
            NODE_SHELL_MIN_DEADLINE_SECONDS,
            NODE_SHELL_MAX_DEADLINE_SECONDS,
        );
        Ok(self)
    }
}

fn is_dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Lifecycle phase, always reported by the backend (FR-007); the UI never assumes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum NodeShellPhase {
    Creating,
    Pending,
    Running,
    Attached,
    Terminating,
    Removed,
    Failed,
}

/// Result of one `SelfSubjectAccessReview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum NodeShellCheckOutcome {
    Allowed,
    Denied {
        reason: Option<String>,
    },
    /// The review itself failed; treated as not allowed (fail safe).
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NodeShellAccessCheck {
    pub verb: String,
    pub resource: String,
    pub subresource: Option<String>,
    pub namespace: Option<String>,
    pub outcome: NodeShellCheckOutcome,
}

impl NodeShellAccessCheck {
    pub fn is_allowed(&self) -> bool {
        matches!(self.outcome, NodeShellCheckOutcome::Allowed)
    }
}

/// A Fathom node-shell pod found by label, not owned by the active session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OrphanedNodeShellPod {
    pub namespace: String,
    pub name: String,
    pub node: Option<String>,
    pub phase: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NodeShellPodRef {
    pub namespace: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_missing_keys() {
        let s: NodeShellSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, NodeShellSettings::default());
        let s: NodeShellSettings = serde_json::from_str(r#"{"image":"alpine"}"#).unwrap();
        assert_eq!(s.image, "alpine");
        assert_eq!(s.namespace, "default");
        assert_eq!(s.deadline_seconds, 14_400);
    }

    #[test]
    fn settings_round_trip() {
        let s = NodeShellSettings {
            image: "x/y:1".to_owned(),
            namespace: "kube-system".to_owned(),
            image_pull_secret: Some("regcred".to_owned()),
            deadline_seconds: 600,
        };
        let back: NodeShellSettings =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn normalization_clamps_and_validates() {
        let s = NodeShellSettings {
            deadline_seconds: 5,
            image: "  alpine ".to_owned(),
            image_pull_secret: Some("  ".to_owned()),
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(s.deadline_seconds, 300);
        assert_eq!(s.image, "alpine");
        assert_eq!(s.image_pull_secret, None);
        let s = NodeShellSettings {
            deadline_seconds: 10_000_000,
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(s.deadline_seconds, 86_400);
        assert!(
            NodeShellSettings {
                image: " ".to_owned(),
                ..Default::default()
            }
            .normalized()
            .is_err()
        );
        assert!(
            NodeShellSettings {
                namespace: "Not_Valid".to_owned(),
                ..Default::default()
            }
            .normalized()
            .is_err()
        );
    }
}

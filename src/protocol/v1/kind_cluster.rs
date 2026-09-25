//! Payload types for in-app local kind cluster provisioning (spec 030).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Starting configuration for a new kind cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KindClusterProfileId {
    /// Single control-plane node, kind defaults.
    Bare,
    /// Operator-supplied kind config YAML.
    CustomYaml,
}

/// Container runtime detected on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KindRuntimeKind {
    Docker,
    Podman,
}

/// Host readiness snapshot (backend-computed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KindPrerequisites {
    pub kind_installed: bool,
    pub kind_version: Option<String>,
    pub runtime_installed: bool,
    pub runtime_kind: Option<KindRuntimeKind>,
    pub runtime_running: bool,
    pub ready: bool,
}

/// Status of a listed local kind cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KindClusterStatus {
    Ready,
    InProgress,
    Error,
}

/// One row of the local kind cluster list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KindClusterSummary {
    pub name: String,
    pub status: KindClusterStatus,
    pub profile: Option<KindClusterProfileId>,
    /// kubeconfig context kind registers for this cluster (`kind-<name>`).
    pub context_name: String,
}

/// Kind of tracked in-flight operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KindClusterOperationKind {
    Create,
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerequisites_roundtrip() {
        let p = KindPrerequisites {
            kind_installed: true,
            kind_version: Some("kind v0.23.0".into()),
            runtime_installed: true,
            runtime_kind: Some(KindRuntimeKind::Podman),
            runtime_running: false,
            ready: false,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<KindPrerequisites>(&json).unwrap(), p);
        assert!(json.contains("\"podman\""));
    }

    #[test]
    fn summary_roundtrip() {
        let s = KindClusterSummary {
            name: "dev".into(),
            status: KindClusterStatus::InProgress,
            profile: Some(KindClusterProfileId::CustomYaml),
            context_name: "kind-dev".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("in_progress") && json.contains("custom_yaml"));
        assert_eq!(
            serde_json::from_str::<KindClusterSummary>(&json).unwrap(),
            s
        );
    }
}

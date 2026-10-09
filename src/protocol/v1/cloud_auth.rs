//! 035-cloud-auth-plugins: typed detection and errors for managed-cloud exec credential
//! plugins (`aws eks get-token`, `gke-gcloud-auth-plugin`, `kubelogin`).
//!
//! Tokens never appear in this module's types (data-model.md): only provider, command, resolved
//! path, and redacted/truncated messages cross the protocol boundary.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Maximum length in bytes of [`PluginError::message`].
pub const PLUGIN_ERROR_MAX_MESSAGE_BYTES: usize = 4096;

/// The managed-cloud provider inferred from a context's exec plugin command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CloudProvider {
    Eks,
    Gke,
    Aks,
    /// An exec plugin Fathom does not recognise.
    GenericExec,
    /// A static-credential context (no exec plugin).
    None,
}

impl CloudProvider {
    /// Human label used in error messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Eks => "EKS",
            Self::Gke => "GKE",
            Self::Aks => "AKS",
            Self::GenericExec => "exec plugin",
            Self::None => "static credentials",
        }
    }
}

/// Whether a context may run exec commands outside the known-provider allowlist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ExecPolicy {
    /// Only `aws eks get-token`, `aws-iam-authenticator`, `gke-gcloud-auth-plugin`, `kubelogin`,
    /// named as a bare command and resolved from `PATH` (a path-bearing `command` is blocked),
    /// with the kubeconfig's `exec.env` reduced to a profile/region/config-file allowlist.
    #[default]
    AllowKnownOnly,
    /// Any exec command in the kubeconfig may run (explicit per-context opt-in).
    AllowAll,
}

/// Detection result for one context (never spawns the plugin).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CloudAuthInfo {
    pub context: String,
    pub provider: CloudProvider,
    pub command: Option<String>,
    /// `None` for static-credential contexts.
    pub installed: Option<bool>,
    pub resolved_path: Option<String>,
    /// False when the command is outside the allowlist and the context is `allowKnownOnly`.
    pub exec_policy_allowed: bool,
    pub exec_policy: ExecPolicy,
    pub install_hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PluginErrorKind {
    PluginMissing,
    PluginFailed,
    PluginTimeout,
    CredentialExpired,
    PluginBlocked,
    PluginInteractiveUnsupported,
}

/// A typed exec-plugin failure. `message` is redacted and at most
/// [`PLUGIN_ERROR_MAX_MESSAGE_BYTES`] bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PluginError {
    pub kind: PluginErrorKind,
    pub context: String,
    pub provider: CloudProvider,
    pub command: String,
    pub message: String,
    pub hint: Option<String>,
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} ({} plugin `{}`, context {}): {}",
            self.kind,
            self.provider.label(),
            self.command,
            self.context,
            self.message
        )?;
        if let Some(hint) = &self.hint {
            write!(f, " -- {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PluginError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: T) {
        let json = serde_json::to_string(&v).expect("serialize");
        assert_eq!(serde_json::from_str::<T>(&json).expect("deserialize"), v);
    }

    #[test]
    fn provider_and_policy_serialize_camel_case() {
        assert_eq!(
            serde_json::to_string(&CloudProvider::GenericExec).unwrap(),
            "\"genericExec\""
        );
        assert_eq!(
            serde_json::to_string(&ExecPolicy::AllowKnownOnly).unwrap(),
            "\"allowKnownOnly\""
        );
        assert_eq!(ExecPolicy::default(), ExecPolicy::AllowKnownOnly);
    }

    #[test]
    fn every_error_kind_round_trips() {
        for kind in [
            PluginErrorKind::PluginMissing,
            PluginErrorKind::PluginFailed,
            PluginErrorKind::PluginTimeout,
            PluginErrorKind::CredentialExpired,
            PluginErrorKind::PluginBlocked,
            PluginErrorKind::PluginInteractiveUnsupported,
        ] {
            rt(PluginError {
                kind,
                context: "prod".into(),
                provider: CloudProvider::Gke,
                command: "gke-gcloud-auth-plugin".into(),
                message: "boom".into(),
                hint: Some("gcloud auth login".into()),
            });
        }
        assert_eq!(
            serde_json::to_string(&PluginErrorKind::PluginInteractiveUnsupported).unwrap(),
            "\"pluginInteractiveUnsupported\""
        );
    }

    #[test]
    fn info_round_trips() {
        rt(CloudAuthInfo {
            context: "c".into(),
            provider: CloudProvider::Aks,
            command: Some("kubelogin".into()),
            installed: Some(false),
            resolved_path: None,
            exec_policy_allowed: true,
            exec_policy: ExecPolicy::AllowAll,
            install_hint: Some("az aks install-cli".into()),
        });
    }

    #[test]
    fn display_names_plugin_provider_context_and_hint() {
        let e = PluginError {
            kind: PluginErrorKind::PluginMissing,
            context: "prod".into(),
            provider: CloudProvider::Gke,
            command: "gke-gcloud-auth-plugin".into(),
            message: "not found".into(),
            hint: Some("install it".into()),
        };
        let s = e.to_string();
        for needle in ["GKE", "gke-gcloud-auth-plugin", "prod", "install it"] {
            assert!(s.contains(needle), "{s}");
        }
    }
}

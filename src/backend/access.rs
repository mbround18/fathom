//! How the backend obtains Kubernetes clients and where it keeps its state (spec
//! `031-hosted-server-mode`, T009).
//!
//! The desktop app and devbridge use [`KubeconfigAccess`]: today's behavior, unchanged. The
//! hosted server supplies its own [`ClusterAccess`] (a per-user client authenticated as the
//! signed-in user, no kubeconfig, no local features) without the backend knowing about OIDC.

use std::path::PathBuf;

use async_trait::async_trait;
use kube::{Client, Config, config::KubeConfigOptions};

use super::kube::load_kubeconfig;

/// The signed-in user behind a hosted backend. Never present on desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Stable OIDC subject.
    pub sub: String,
    pub username: String,
    pub groups: Vec<String>,
    pub email: Option<String>,
}

impl Identity {
    /// The audit-log attribution for this identity.
    pub fn audit_actor(&self) -> crate::protocol::AuditActor {
        crate::protocol::AuditActor {
            sub: self.sub.clone(),
            username: self.username.clone(),
        }
    }
}

/// Source of Kubernetes clients and state location for one backend instance.
#[async_trait]
pub trait ClusterAccess: Send + Sync {
    /// Build a client for `context` (`None` = the access's default context).
    async fn build_client(&self, context: Option<&str>) -> anyhow::Result<Client>;

    /// The context the backend starts on when nothing was selected. `None` = whatever the
    /// kubeconfig's current context is (desktop). Hosted returns its fixed home context.
    fn home_context(&self) -> Option<String> {
        None
    }

    /// Whether kind provisioning, kubeconfig/context switching, stdio MCP, fleet and other
    /// machine-local features make sense. `false` for the hosted server.
    fn allows_local_features(&self) -> bool {
        true
    }

    /// Override for the `~/.fathom` state directory. `None` = the default.
    fn state_root(&self) -> Option<PathBuf> {
        None
    }
}

/// Default access: kubeconfig contexts, falling back to kube-rs's inferred config. This is
/// exactly what `connect_client_for_current_context` did before the seam existed.
#[derive(Debug, Clone, Copy, Default)]
pub struct KubeconfigAccess;

#[async_trait]
impl ClusterAccess for KubeconfigAccess {
    async fn build_client(&self, context: Option<&str>) -> anyhow::Result<Client> {
        // If we have a selected context and kubeconfig exists, use it; otherwise fall back.
        if let Some(ctx) = context
            && let Ok(kubeconfig) = load_kubeconfig()
        {
            let opts = KubeConfigOptions {
                context: Some(ctx.to_owned()),
                ..Default::default()
            };
            let config = Config::from_custom_kubeconfig(kubeconfig, &opts).await?;
            return Ok(Client::try_from(config)?);
        }
        Ok(Client::try_default().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_access_is_desktop_behavior() {
        let a = KubeconfigAccess;
        assert!(a.home_context().is_none());
        assert!(a.allows_local_features());
        assert!(a.state_root().is_none());
    }

    #[test]
    fn identity_maps_to_audit_actor_without_groups() {
        let id = Identity {
            sub: "s".into(),
            username: "alice".into(),
            groups: vec!["g".into()],
            email: None,
        };
        let actor = id.audit_actor();
        assert_eq!(
            (actor.sub.as_str(), actor.username.as_str()),
            ("s", "alice")
        );
    }
}

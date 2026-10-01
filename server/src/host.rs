//! The real [`TenantHost`] (spec 031): per-user backends from [`TenantRegistry`], per-user
//! [`TokenCell`]s fed by the session's tokens, one refresh task per session, and teardown when a
//! user's last session ends.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use fathom::backend::access::Identity as BackendIdentity;
use fathom::backend::audit_log::AuditSink;
use fathom::protocol::{
    BackendCmd, BridgeError, DevBridgeServerMsg, EventBatch, NotificationLevel, PublicLimits,
    RequestId, SubscriptionInfo, SystemEvent,
};
use kube::Config;
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::audit::HostedAuditSink;
use crate::config::{AccessModel, HostedConfig, KubeTokenKind};
use crate::kubeauth::{self, AuthError, Credentials, Impersonation, TokenCell};
use crate::oidc::OidcClient;
use crate::oidc_seed;
use crate::refresh::{self, RefreshConfig, RefreshParams};
use crate::session::{Identity, Session, SessionStore, Tokens};
use crate::tenant::{
    HOME_CONTEXT, HostedAccess, SpawnParams, TabAttachment, TenantRegistry, UserBackend,
};
use crate::ws::{AttachError, TenantHost, UserConn};

/// The kube bearer for `tokens` under the configured token kind.
pub fn kube_bearer(kind: KubeTokenKind, tokens: &Tokens) -> &secrecy::SecretString {
    match kind {
        KubeTokenKind::Id => &tokens.id_token,
        KubeTokenKind::Access => &tokens.access_token,
    }
}

fn token_expiry(tokens: &Tokens) -> SystemTime {
    SystemTime::now()
        + tokens
            .expires_at
            .saturating_duration_since(tokio::time::Instant::now())
}

/// How the user's identity reaches the API server. Impersonation needs at least one group.
pub fn credentials_for(
    cfg: &HostedConfig,
    identity: &Identity,
    cell: &TokenCell,
) -> Result<Credentials, String> {
    match cfg.access_model {
        AccessModel::PassThrough => Ok(Credentials::Bearer(cell.clone())),
        AccessModel::Impersonation => {
            if identity.groups.is_empty() {
                return Err(format!(
                    "no groups: the `{}` claim is empty, so there is nothing to impersonate; \
                     ask an administrator to map groups in the identity provider",
                    cfg.claims.groups
                ));
            }
            let user = format!("{}{}", cfg.claims.username_prefix, identity.username);
            let groups: Vec<String> = identity
                .groups
                .iter()
                .map(|g| format!("{}{g}", cfg.claims.groups_prefix))
                .collect();
            Impersonation::new(&user, &groups)
                .map(Credentials::Impersonate)
                .map_err(|e: AuthError| e.to_string())
        }
    }
}

/// Namespaces the zero-permission probe checks in addition to cluster scope: `default`, the
/// server's own namespace, and any listed in `FATHOM_ACCESS_PROBE_NAMESPACES` (comma
/// separated; Helm `accessProbeNamespaces`). A user whose only access is a `RoleBinding` in
/// some other namespace would otherwise be shown the no-access state (a false positive).
pub fn probe_namespaces() -> Vec<String> {
    let own =
        std::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/namespace").ok();
    let extra = std::env::var("FATHOM_ACCESS_PROBE_NAMESPACES").ok();
    probe_namespaces_from(own.as_deref(), extra.as_deref())
}

fn probe_namespaces_from(own: Option<&str>, extra: Option<&str>) -> Vec<String> {
    let mut v = vec!["default".to_owned()];
    let listed = extra.unwrap_or_default().split(',');
    for ns in own.into_iter().chain(listed).map(str::trim) {
        if !ns.is_empty() && !v.iter().any(|n| n == ns) {
            v.push(ns.to_owned());
        }
    }
    v
}

pub struct HostedTenantHost {
    cfg: Arc<HostedConfig>,
    oidc: Arc<OidcClient>,
    sessions: SessionStore,
    registry: TenantRegistry,
    base: Result<Config, String>,
    cells: Mutex<HashMap<String, TokenCell>>,
    refreshers: Mutex<HashMap<String, JoinHandle<()>>>,
    client: tokio::sync::OnceCell<kube::Client>,
    refresh_cfg: RefreshConfig,
    /// One attributed audit log shared by every user's backend (single writer).
    audit: Arc<dyn AuditSink>,
}

impl HostedTenantHost {
    /// Build from the in-cluster config. A missing in-cluster environment does not abort
    /// startup; the host then reports not-ready and refuses tabs.
    pub fn new(cfg: Arc<HostedConfig>, oidc: Arc<OidcClient>, sessions: SessionStore) -> Arc<Self> {
        let pass = cfg.access_model == AccessModel::PassThrough;
        let base = kubeauth::incluster_config(pass).map_err(|e| e.to_string());
        Self::with_base(cfg, oidc, sessions, base)
    }

    pub fn with_base(
        cfg: Arc<HostedConfig>,
        oidc: Arc<OidcClient>,
        sessions: SessionStore,
        base: Result<Config, String>,
    ) -> Arc<Self> {
        let refresh_cfg = RefreshConfig {
            grace: cfg.refresh_grace,
            revalidate: cfg.revalidate,
            recycle: Duration::from_secs(cfg.limits.stream_recycle_minutes * 60),
            ..RefreshConfig::default()
        };
        let audit: Arc<dyn AuditSink> = Arc::new(HostedAuditSink::new(
            &cfg.state_dir,
            cfg.audit_retention,
            cfg.audit_viewer_groups.clone(),
        ));
        let host = Arc::new(Self {
            audit,
            cfg,
            oidc,
            sessions,
            registry: TenantRegistry::new(),
            base,
            cells: Mutex::default(),
            refreshers: Mutex::default(),
            client: tokio::sync::OnceCell::new(),
            refresh_cfg,
        });
        host.start();
        host
    }

    pub fn registry(&self) -> &TenantRegistry {
        &self.registry
    }

    /// Watch session endings (sign-out, expiry, revocation, eviction) and sweep idle backends.
    fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let mut ended = self.sessions.subscribe_ended();
        tokio::spawn(async move {
            loop {
                let ev = ended.recv().await;
                let Some(host) = weak.upgrade() else { return };
                match ev {
                    Ok(e) => host.on_ended(&e.id, &e.user_key),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => host.reconcile(),
                    Err(_) => return,
                }
            }
        });
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                let Some(host) = weak.upgrade() else { return };
                let park = Duration::from_secs(host.cfg.limits.idle_park_minutes * 60);
                let drop_after = host.cfg.session_idle.max(park);
                let h = Arc::clone(&host);
                let _ =
                    tokio::task::spawn_blocking(move || h.registry.sweep(park, drop_after)).await;
            }
        });
    }

    fn on_ended(self: &Arc<Self>, session_id: &str, user_key: &str) {
        if let Some(h) = self
            .refreshers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id)
        {
            h.abort();
        }
        if self.sessions.count_for_user(user_key) == 0 {
            self.end_user(user_key);
        }
    }

    /// After missed notifications: end every backend whose user has no session left.
    fn reconcile(self: &Arc<Self>) {
        let keys: Vec<String> = self
            .cells
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for k in keys {
            if self.sessions.count_for_user(&k) == 0 {
                self.end_user(&k);
            }
        }
    }

    fn end_user(self: &Arc<Self>, user_key: &str) {
        if let Some(cell) = self
            .cells
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(user_key)
        {
            cell.revoke();
        }
        let host = Arc::clone(self);
        let key = user_key.to_owned();
        // Ending joins backend threads: keep that off the async workers.
        tokio::task::spawn_blocking(move || host.registry.end(&key));
    }

    fn state_root(&self, user_key: &str) -> PathBuf {
        self.cfg.state_dir.join("users").join(user_key)
    }

    fn backend_for(&self, session: &Session) -> Result<Arc<UserBackend>, AttachError> {
        let base = self
            .base
            .clone()
            .map_err(|e| AttachError::Unavailable(format!("no in-cluster config: {e}")))?;
        let cell = self
            .cells
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(session.user_key.clone())
            .or_default()
            .clone();
        self.registry
            .get_or_create(&session.user_key, || {
                // A backend that was ended revoked the shared cell; seed it from this session.
                cell.set(
                    kube_bearer(self.cfg.kube_token, &session.tokens),
                    Some(token_expiry(&session.tokens)),
                )?;
                let creds = credentials_for(&self.cfg, &session.identity, &cell)
                    .map_err(anyhow::Error::msg)?;
                let access = Arc::new(HostedAccess::new(
                    base.clone(),
                    creds,
                    self.state_root(&session.user_key),
                ));
                UserBackend::spawn(SpawnParams {
                    user_key: session.user_key.clone(),
                    identity: BackendIdentity {
                        sub: session.identity.sub.clone(),
                        username: session.identity.username.clone(),
                        groups: session.identity.groups.clone(),
                        email: session.identity.email.clone(),
                    },
                    access,
                    token_cell: Some(cell.clone()),
                    limits: self.cfg.limits.clone(),
                    audit: self.audit.clone(),
                })
            })
            .map_err(|e| AttachError::Unavailable(e.to_string()))
    }

    /// A kube client carrying this session's own identity. FR-034's review is performed as the
    /// signed-in user — the same credentials every other read of theirs goes out with — never
    /// with the server's, and never against a Fathom-internal role list.
    fn user_client(&self, session: &Session) -> Result<kube::Client, String> {
        let base = self
            .base
            .clone()
            .map_err(|e| format!("no in-cluster config: {e}"))?;
        let cell = self
            .cells
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session.user_key)
            .cloned()
            .ok_or_else(|| "no kube credential for this session".to_owned())?;
        let creds = credentials_for(&self.cfg, &session.identity, &cell)?;
        kubeauth::build_client(base, creds).map_err(|e| e.to_string())
    }

    /// Put the server's own validated OIDC configuration in front of this tab as a seeded,
    /// read-only issuer (FR-031, FR-032) — but only once the API server has said this user may
    /// read the object it comes from (FR-034). Anything else refuses, in the API server's own
    /// words (FR-035).
    ///
    /// Off the attach path, because the review is a round trip and the tab's frames start
    /// flowing only after it subscribes. Run on every attach rather than once per user: the
    /// fanout hub keeps a backlog only while no tab is attached, so a tab opened later would
    /// otherwise never see the seed. The issuer snapshot and the refusal are both idempotent.
    fn spawn_oidc_seed(&self, session: &Session, backend: &Arc<UserBackend>) {
        let object = oidc_seed::SeedObjectRef::from_env();
        let client = self.user_client(session);
        let cfg = Arc::clone(&self.cfg);
        let backend = Arc::clone(backend);
        tokio::spawn(async move {
            let object = match object {
                Ok(o) => o,
                // Without the object there is nothing to review against, and an unreviewed
                // surface is a refused surface: fail closed and state the unknown verbatim.
                Err(reason) => {
                    backend.emit_system(SystemEvent::Notification {
                        level: NotificationLevel::Warn,
                        message: format!(
                            "Fathom's OIDC configuration is not shown: it could not be checked, \
                             so it is treated as not permitted — {reason}"
                        ),
                    });
                    return;
                }
            };
            let verdict = match client {
                Ok(c) => oidc_seed::review_config_access(c, &object).await,
                // A credential we cannot even build is an undetermined review, not a denial.
                Err(reason) => oidc_seed::decide(Err(reason), &object),
            };
            if !verdict.allowed {
                backend.emit_system(oidc_seed::denied_notice(&verdict, &object));
                return;
            }
            for event in oidc_seed::seed_events(&cfg, &object) {
                backend.emit_system(event);
            }
        });
    }

    fn ensure_refresher(&self, session: &Session, backend: &Arc<UserBackend>) {
        let mut r = self.refreshers.lock().unwrap_or_else(|e| e.into_inner());
        if r.get(&session.id).is_some_and(|h| !h.is_finished()) {
            return;
        }
        let Some(cell) = self
            .cells
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session.user_key)
            .cloned()
        else {
            return;
        };
        let handle = refresh::spawn(RefreshParams {
            session_id: session.id.clone(),
            user_key: session.user_key.clone(),
            refresh_token: session.tokens.refresh_token.clone(),
            expires_at: session.tokens.expires_at,
            cell,
            kube_token: self.cfg.kube_token,
            store: Arc::new(self.sessions.clone()),
            refresher: self.oidc.clone(),
            backend: backend.clone(),
            cfg: self.refresh_cfg,
        });
        r.insert(session.id.clone(), handle);
    }
}

#[async_trait]
impl TenantHost for HostedTenantHost {
    async fn ready(&self) -> Result<(), String> {
        let base = self
            .base
            .clone()
            .map_err(|e| format!("no in-cluster config: {e}"))?;
        let client = self
            .client
            .get_or_try_init(|| async { kube::Client::try_from(base) })
            .await
            .map_err(|e| format!("kube client: {e}"))?;
        match tokio::time::timeout(Duration::from_secs(3), client.apiserver_version()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("home cluster unreachable: {e}")),
            Err(_) => Err("home cluster unreachable: timed out".into()),
        }
    }

    fn home_context(&self) -> String {
        HOME_CONTEXT.to_owned()
    }

    fn public_limits(&self) -> PublicLimits {
        self.cfg.limits.public()
    }

    async fn attach(
        &self,
        session: &Session,
        frames: UnboundedSender<String>,
    ) -> Result<Box<dyn UserConn>, AttachError> {
        let backend = self.backend_for(session)?;
        let attachment = backend.attach_tab().map_err(AttachError::Limit)?;
        self.ensure_refresher(session, &backend);
        self.spawn_oidc_seed(session, &backend);
        Ok(Box::new(HostedConn::new(backend, attachment, frames)))
    }
}

/// One tab. A forwarder task owns the [`TabAttachment`] and turns its batches into
/// `{"op":"batch"}` frames once the tab has subscribed; dropping the conn stops it, which
/// detaches the tab and frees its slot.
struct HostedConn {
    backend: Arc<UserBackend>,
    subscribed: Arc<Notify>,
    task: JoinHandle<()>,
}

impl HostedConn {
    fn new(
        backend: Arc<UserBackend>,
        mut attachment: TabAttachment,
        frames: UnboundedSender<String>,
    ) -> Self {
        let subscribed = Arc::new(Notify::new());
        let gate = Arc::clone(&subscribed);
        let task = tokio::spawn(async move {
            gate.notified().await;
            while let Some(b) = attachment.rx.recv().await {
                let msg = DevBridgeServerMsg::Batch {
                    batch: EventBatch {
                        envelopes: b.envelopes,
                        dropped: b.dropped,
                    },
                };
                match serde_json::to_string(&msg) {
                    Ok(json) => {
                        if frames.send(json).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::error!("host: failed to serialize batch: {e}"),
                }
            }
        });
        Self {
            backend,
            subscribed,
            task,
        }
    }
}

impl Drop for HostedConn {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl UserConn for HostedConn {
    fn send(&self, cmd: BackendCmd) -> Result<RequestId, BridgeError> {
        self.backend.send(cmd)
    }
    fn subscription_info(&self) -> Result<SubscriptionInfo, BridgeError> {
        self.backend.subscription_info()
    }
    fn subscribe(&self) -> Result<(), BridgeError> {
        self.subscribed.notify_one();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn probe_namespaces_add_own_and_configured_without_duplicates() {
        assert_eq!(probe_namespaces_from(None, None), vec!["default"]);
        assert_eq!(
            probe_namespaces_from(Some("fathom\n"), Some(" demo,,default, team-a ")),
            vec!["default", "fathom", "demo", "team-a"]
        );
    }

    #[test]
    fn kube_bearer_follows_the_configured_token_kind() {
        let t = Tokens {
            id_token: secrecy::SecretString::from("id".to_owned()),
            access_token: secrecy::SecretString::from("access".to_owned()),
            refresh_token: None,
            expires_at: tokio::time::Instant::now(),
        };
        // The kind cluster's API server validates OIDC ID tokens (the default).
        assert_eq!(kube_bearer(KubeTokenKind::Id, &t).expose_secret(), "id");
        assert_eq!(
            kube_bearer(KubeTokenKind::Access, &t).expose_secret(),
            "access"
        );
    }
}

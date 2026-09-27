use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, KubeconfigIssue, KubeconfigIssueKind, Route,
    SystemEvent, section_keys,
};
use crossbeam_channel::Sender;
use kube::{
    Client, Config,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;

use super::oidc::session::TokenSource;
use super::{events, state::BackendState};

/// Load the kubeconfig the way `kubectl` does: the files listed in `KUBECONFIG` (merged, missing
/// entries skipped) when that variable is set and non-empty, otherwise `~/.kube/config`.
///
/// A malformed file is an error, never silently skipped. When `KUBECONFIG` is set Fathom never
/// falls back to `~/.kube/config`: a user who points `KUBECONFIG` somewhere must not end up
/// connected to whatever their home kubeconfig's current context is.
///
/// `pub(super)` so `context_status.rs` can build short-lived clients for non-active contexts
/// (012-incluster-daemon-federation P1) using the same kubeconfig-resolution path as the active
/// connection.
pub(super) fn load_kubeconfig() -> anyhow::Result<Kubeconfig> {
    resolve_kubeconfig(std::env::var_os("KUBECONFIG"), dirs::home_dir())
        .map(|(kc, _)| kc)
        .map_err(|issue| anyhow::anyhow!("{}", super::first_run::describe_issue(&issue)))
}

/// The kubeconfig search path: the non-empty `KUBECONFIG` entries, or `~/.kube/config`.
pub(super) fn kubeconfig_search_paths(
    env: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Vec<PathBuf> {
    let from_env: Vec<PathBuf> = env
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    if !from_env.is_empty() {
        return from_env;
    }
    home.map(|h| vec![h.join(".kube").join("config")])
        .unwrap_or_default()
}

/// Resolve and parse the kubeconfig from an explicit `KUBECONFIG` value and home directory (pure
/// apart from file reads, so it is unit-testable). Returns the merged config and the files it
/// came from, or the classified problem.
pub(super) fn resolve_kubeconfig(
    env: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Result<(Kubeconfig, Vec<PathBuf>), KubeconfigIssue> {
    let paths = kubeconfig_search_paths(env, home);
    let mut merged: Option<Kubeconfig> = None;
    let mut used = Vec::new();
    for path in &paths {
        match std::fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(issue(KubeconfigIssueKind::Unreadable, path, e.to_string())),
            Ok(meta) if !meta.is_file() => {
                return Err(issue(
                    KubeconfigIssueKind::Unreadable,
                    path,
                    "not a regular file".to_owned(),
                ));
            }
            Ok(_) => {}
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| issue(KubeconfigIssueKind::Unreadable, path, e.to_string()))?;
        if text.trim().is_empty() {
            // An empty file is a valid, empty kubeconfig (kubectl treats it the same way).
            used.push(path.clone());
            merged.get_or_insert_with(Kubeconfig::default);
            continue;
        }
        let kc = Kubeconfig::read_from(path).map_err(|e| {
            issue(
                KubeconfigIssueKind::Malformed,
                path,
                super::first_run::short_detail(&e.to_string()),
            )
        })?;
        used.push(path.clone());
        merged = Some(match merged {
            None => kc,
            Some(m) => m.merge(kc).map_err(|e| {
                issue(
                    KubeconfigIssueKind::Malformed,
                    path,
                    super::first_run::short_detail(&e.to_string()),
                )
            })?,
        });
    }
    let Some(kc) = merged else {
        return Err(KubeconfigIssue {
            kind: KubeconfigIssueKind::Missing,
            paths: paths.iter().map(|p| p.display().to_string()).collect(),
            detail: String::new(),
        });
    };
    if kc.contexts.is_empty() {
        return Err(KubeconfigIssue {
            kind: KubeconfigIssueKind::Empty,
            paths: used.iter().map(|p| p.display().to_string()).collect(),
            detail: String::new(),
        });
    }
    Ok((kc, used))
}

fn issue(kind: KubeconfigIssueKind, path: &std::path::Path, detail: String) -> KubeconfigIssue {
    KubeconfigIssue {
        kind,
        paths: vec![path.display().to_string()],
        detail,
    }
}

/// The problem with the effective current context, if any: none selected, or one the
/// kubeconfig does not define.
pub(super) fn current_context_issue(
    contexts: &[String],
    current: Option<&str>,
    paths: &[PathBuf],
) -> Option<KubeconfigIssue> {
    let paths = paths.iter().map(|p| p.display().to_string()).collect();
    match current {
        None => Some(KubeconfigIssue {
            kind: KubeconfigIssueKind::NoCurrentContext,
            paths,
            detail: String::new(),
        }),
        Some(c) if !contexts.iter().any(|x| x == c) => Some(KubeconfigIssue {
            kind: KubeconfigIssueKind::NoCurrentContext,
            paths,
            detail: format!("the current context \"{c}\" is not defined in the kubeconfig"),
        }),
        Some(_) => None,
    }
}

/// Path of the kubeconfig file `load_kubeconfig` reads first (the first readable `KUBECONFIG`
/// entry, else `~/.kube/config`), so subprocesses such as `helm` resolve context names from the
/// same file.
pub(super) fn kubeconfig_path() -> Option<PathBuf> {
    resolve_kubeconfig(std::env::var_os("KUBECONFIG"), dirs::home_dir())
        .ok()
        .and_then(|(_, used)| used.into_iter().next())
}

/// The result of (re)loading the kubeconfig contexts.
pub(super) struct ContextsLoad {
    pub contexts: Vec<String>,
    pub current: Option<String>,
    /// Why there is nothing to connect to, when that is the case.
    pub issue: Option<KubeconfigIssue>,
}

/// Load the contexts and classify any kubeconfig problem, emitting `ContextsUpdated` then
/// `KubeconfigStatus`.
pub(super) fn load_and_send_contexts_full(
    seq: &AtomicU64,
    selected_context: Option<String>,
    sender: &Sender<BackendEventEnvelope>,
) -> ContextsLoad {
    tracing::info!("Loading kubeconfig contexts...");
    let load = match resolve_kubeconfig(std::env::var_os("KUBECONFIG"), dirs::home_dir()) {
        Ok((kc, used)) => {
            let (contexts, current) = contexts_from_kubeconfig(&kc, selected_context);
            let issue = current_context_issue(&contexts, current.as_deref(), &used);
            // A current-context that names a missing context is not something to connect to.
            let current = current.filter(|c| contexts.contains(c));
            ContextsLoad {
                contexts,
                current,
                issue,
            }
        }
        Err(issue) => {
            tracing::warn!(
                "No usable kubeconfig: {}",
                super::first_run::describe_issue(&issue)
            );
            ContextsLoad {
                contexts: Vec::new(),
                current: None,
                issue: Some(issue),
            }
        }
    };
    emit_contexts(seq, sender, load.contexts.clone(), load.current.clone());
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::KubeconfigStatus {
            issue: load.issue.clone(),
        }),
    );
    load
}

/// Loads and emits `ContextsUpdated`, returning the `(contexts, current)` tuple it computed so
/// callers can also trigger per-context status probes (012-incluster-daemon-federation P1)
/// without re-parsing the kubeconfig.
pub(super) fn load_and_send_contexts(
    seq: &AtomicU64,
    selected_context: Option<String>,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<(Vec<String>, Option<String>)> {
    let load = load_and_send_contexts_full(seq, selected_context, sender);
    Ok((load.contexts, load.current))
}

/// Like [`load_and_send_contexts`], but a backend without local features (the hosted server,
/// 031) has no kubeconfig: its only context is the access's fixed home context. Without this
/// the hosted UI would see an empty context list, which it reads as "no access" for everyone.
pub(super) fn send_contexts_for(
    access: &dyn super::access::ClusterAccess,
    seq: &AtomicU64,
    selected_context: Option<String>,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<(Vec<String>, Option<String>)> {
    if !access.allows_local_features()
        && let Some(home) = access.home_context()
    {
        let contexts = vec![home.clone()];
        emit_contexts(seq, sender, contexts.clone(), Some(home.clone()));
        return Ok((contexts, Some(home)));
    }
    load_and_send_contexts(seq, selected_context, sender)
}

/// Sorted context names plus the effective current context (explicit selection wins
/// over kubeconfig's `current-context`).
fn contexts_from_kubeconfig(
    kubeconfig: &Kubeconfig,
    selected_context: Option<String>,
) -> (Vec<String>, Option<String>) {
    tracing::info!(
        "Kubeconfig loaded successfully, found {} contexts",
        kubeconfig.contexts.len()
    );

    let mut contexts: Vec<String> = kubeconfig.contexts.iter().map(|c| c.name.clone()).collect();
    contexts.sort();
    tracing::info!("Sending {} contexts to UI: {:?}", contexts.len(), contexts);

    let current = selected_context.or_else(|| kubeconfig.current_context.clone());
    tracing::info!("Current context: {:?}", current);
    (contexts, current)
}

fn emit_contexts(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    contexts: Vec<String>,
    current: Option<String>,
) {
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ContextsUpdated { contexts, current }),
    );
}

pub(super) async fn connect_client_for_current_context(
    state: &BackendState,
) -> anyhow::Result<Client> {
    // 031: a hosted server supplies its own per-user access (no kubeconfig, no local auth).
    if !state.access.allows_local_features() {
        return state
            .access
            .build_client(state.cluster.current_context.as_deref())
            .await;
    }
    // Local mode: always the kubeconfig (never an in-cluster/`try_default` guess, whose
    // "Failed to infer configuration" wall is no use to someone without a kubeconfig).
    let Some(ctx) = state.cluster.current_context.clone() else {
        anyhow::bail!("No kubeconfig context is selected.");
    };
    let kubeconfig = load_kubeconfig()?;
    let opts = KubeConfigOptions {
        context: Some(ctx.clone()),
        ..Default::default()
    };
    let mut config = Config::from_custom_kubeconfig(kubeconfig, &opts).await?;
    // 033 US2: a context opted into OIDC authenticates with the signed-in ID token only.
    if let Some(issuer_id) = oidc_issuer_for_context(&ctx) {
        return build_oidc_client(config, state.oidc.token_source(&issuer_id));
    }
    // 035: exec plugins (EKS/GKE/AKS) run through the shared, bounded credential cache.
    super::cloud_auth::prepare_config(&ctx, &mut config).await?;
    Ok(Client::try_from(config)?)
}

/// The issuer a context is opted into, or `None` for the default kubeconfig-only path.
pub(super) fn oidc_issuer_for_context(context: &str) -> Option<String> {
    use crate::protocol::v1::ContextAuthMode;
    super::persistence::load_oidc_config()
        .contexts
        .into_iter()
        .find(|c| c.context_name == context && c.mode == ContextAuthMode::Oidc)
        .and_then(|c| c.issuer_id)
}

/// Tower layer that stamps `Authorization: Bearer <current ID token>` on every request.
///
/// The token is read per request (the refresh task swaps it under us). When there is no token
/// the request fails with the reason; it is never sent unauthenticated and never re-signed with
/// kubeconfig credentials (033 FR: no fallback after OIDC failure).
#[derive(Clone)]
pub struct OidcBearerLayer {
    source: TokenSource,
}

impl OidcBearerLayer {
    pub fn new(source: TokenSource) -> Self {
        Self { source }
    }
}

impl<S> tower::Layer<S> for OidcBearerLayer {
    type Service = OidcBearer<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OidcBearer {
            inner,
            source: self.source.clone(),
        }
    }
}

#[derive(Clone)]
pub struct OidcBearer<S> {
    inner: S,
    source: TokenSource,
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

impl<S, B> tower::Service<http::Request<B>> for OidcBearer<S>
where
    S: tower::Service<http::Request<B>>,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    S::Response: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = futures::future::BoxFuture<'static, Result<S::Response, BoxError>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: http::Request<B>) -> Self::Future {
        match (self.source)() {
            Ok(token) => {
                match http::HeaderValue::from_str(&format!("Bearer {token}")) {
                    Ok(mut value) => {
                        value.set_sensitive(true);
                        req.headers_mut().insert(http::header::AUTHORIZATION, value);
                    }
                    Err(_) => {
                        return Box::pin(async {
                            Err("the OIDC ID token is not a valid header value".into())
                        });
                    }
                }
                let fut = self.inner.call(req);
                Box::pin(async move { fut.await.map_err(Into::into) })
            }
            Err(reason) => Box::pin(async move { Err(reason.into()) }),
        }
    }
}

/// Build a client for an OIDC-opted context: kubeconfig credentials are discarded and the
/// bearer layer is the only authentication.
pub fn build_oidc_client(mut config: Config, source: TokenSource) -> anyhow::Result<Client> {
    config.auth_info = kube::config::AuthInfo::default();
    let client = kube::client::ClientBuilder::try_from(config)?
        .with_layer(&OidcBearerLayer::new(source))
        .build();
    Ok(client)
}

/// List `K` in the namespaces of `filter` (one namespaced list each), or cluster-wide when the
/// filter is empty. A user whose permissions are namespace-scoped (a `RoleBinding`, common
/// on the hosted server, 031) can list inside their namespace but not at cluster scope.
pub(super) async fn list_in_filter<K>(client: Client, filter: &[String]) -> kube::Result<Vec<K>>
where
    K: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>
        + Clone
        + serde::de::DeserializeOwned
        + std::fmt::Debug,
    K::DynamicType: Default,
{
    let params = kube::api::ListParams::default();
    if filter.is_empty() {
        return Ok(kube::Api::<K>::all(client).list(&params).await?.items);
    }
    let mut items = Vec::new();
    for ns in filter {
        let api = kube::Api::<K>::namespaced(client.clone(), ns);
        items.extend(api.list(&params).await?.items);
    }
    Ok(items)
}

pub(super) fn read_kubeconfig_current_context() -> Option<String> {
    load_kubeconfig().ok().and_then(|kc| kc.current_context)
}

#[cfg(test)]
mod hosted_context_tests {
    use super::*;
    use crate::backend::access::ClusterAccess;
    use async_trait::async_trait;

    struct HomeOnly;

    #[async_trait]
    impl ClusterAccess for HomeOnly {
        async fn build_client(&self, _context: Option<&str>) -> anyhow::Result<Client> {
            anyhow::bail!("not used")
        }
        fn home_context(&self) -> Option<String> {
            Some("in-cluster".into())
        }
        fn allows_local_features(&self) -> bool {
            false
        }
    }

    #[test]
    fn hosted_access_reports_its_home_context_without_a_kubeconfig() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let seq = AtomicU64::new(0);
        let (contexts, current) = send_contexts_for(&HomeOnly, &seq, None, &tx).unwrap();
        assert_eq!(contexts, vec!["in-cluster".to_owned()]);
        assert_eq!(current.as_deref(), Some("in-cluster"));
        let env = rx.try_recv().unwrap();
        assert!(matches!(
            env.event,
            BackendEvent::System(SystemEvent::ContextsUpdated { ref contexts, current: Some(ref c) })
                if contexts == &["in-cluster".to_owned()] && c == "in-cluster"
        ));
    }
}

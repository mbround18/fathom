//! 035-cloud-auth-plugins: managed-cloud exec credential plugins (EKS / GKE / AKS).
//!
//! Fathom owns plugin invocation (research.md R1): a kubeconfig `exec` entry is replaced, per
//! `kube::Config`, by a bearer token (or client certificate) obtained through the shared
//! [`cache`] and the bounded [`exec`] runner. kube-rs is therefore never asked to spawn the
//! plugin itself, so a fresh `Client` per fleet poll or probe does not re-run it.
//!
//! Injection mechanism (T004): kube 2.0.1's `AuthInfo.token` (a `SecretString`) makes the client
//! send `Authorization: Bearer`; clearing `AuthInfo.exec` stops kube from spawning anything.
//! Tokens never leave this module except into that field.

pub mod cache;
pub mod detect;
pub mod exec;
pub mod redact;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crossbeam_channel::Sender;
use kube::{Client, Config, config::KubeConfigOptions};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, CloudAuthInfo, CloudProvider, Correlation, ExecPolicy,
    PLUGIN_ERROR_MAX_MESSAGE_BYTES, PluginError, PluginErrorKind, Route, SystemEvent, section_keys,
};

use super::events;
use super::kube::load_kubeconfig;
use cache::CredentialCache;
use exec::{ExecSpec, run_plugin, to_plugin_error};

static CACHE: OnceLock<CredentialCache> = OnceLock::new();

/// The process-wide credential cache shared by connect, probe, and fleet poll (FR-006).
pub fn global_cache() -> &'static CredentialCache {
    CACHE.get_or_init(CredentialCache::default)
}

/// The typed plugin error anywhere in an `anyhow` chain, if that is what failed.
pub fn plugin_error_of(err: &anyhow::Error) -> Option<&PluginError> {
    err.downcast_ref::<PluginError>()
        .or_else(|| err.chain().find_map(|e| e.downcast_ref::<PluginError>()))
}

/// Whether `context` is plugin-backed and had a cached credential, which is now dropped so the
/// next attempt re-invokes the plugin (401 handling, FR-005).
pub fn invalidate_context(context: &str) -> bool {
    global_cache().invalidate_context(context)
}

/// A 401 that persisted after one fresh plugin invocation: surface it as `credentialExpired`
/// (spec US1.3) with the provider's re-login hint, keeping the API error in the chain.
pub fn rejected_after_retry(context: &str, api_error: kube::Error) -> anyhow::Error {
    let exec_cfg = load_kubeconfig()
        .ok()
        .and_then(|kc| detect::exec_for_context(&kc, context));
    let Some(exec_cfg) = exec_cfg else {
        return api_error.into();
    };
    let provider = detect::classify_exec(&exec_cfg);
    let command = exec_cfg.command.clone().unwrap_or_default();
    let plugin = PluginError {
        kind: PluginErrorKind::CredentialExpired,
        context: context.to_owned(),
        provider,
        message: format!("the API server rejected the credential returned by `{command}` (401)"),
        command,
        hint: Some(detect::relogin_hint(provider).to_owned()),
    };
    anyhow::Error::new(api_error).context(plugin)
}

fn blocked_error(context: &str, provider: CloudProvider, command: &str) -> PluginError {
    PluginError {
        kind: PluginErrorKind::PluginBlocked,
        context: context.to_owned(),
        provider,
        command: command.to_owned(),
        message: redact::truncate(
            &format!(
                "`{command}` is not a known cloud auth plugin and this context's exec policy is allowKnownOnly, so it was not run"
            ),
            PLUGIN_ERROR_MAX_MESSAGE_BYTES,
        ),
        hint: Some(
            "Only if you trust this kubeconfig, allow any exec command for this context in the context inspector"
                .to_owned(),
        ),
    }
}

/// Everything [`prepare_config_with`] depends on, so tests can substitute each piece.
pub struct PrepareEnv<'a> {
    pub policy: ExecPolicy,
    pub search: &'a [PathBuf],
    pub cache: &'a CredentialCache,
    pub timeout: Duration,
}

/// Replace `config`'s exec entry with a cached-or-fresh credential. A no-op for contexts
/// without an exec plugin (FR-011: static contexts are untouched).
pub async fn prepare_config_with(
    context: &str,
    config: &mut Config,
    env: &PrepareEnv<'_>,
) -> Result<(), PluginError> {
    let Some(exec_cfg) = config.auth_info.exec.clone() else {
        return Ok(());
    };
    let Some(spec) = ExecSpec::from_kube(&exec_cfg) else {
        return Ok(());
    };
    let provider = detect::classify_exec(&exec_cfg);
    if !detect::policy_allows(provider, env.policy) {
        return Err(blocked_error(context, provider, &spec.command));
    }

    let credential = env
        .cache
        .get_or_fetch(context, spec.identity(), || async {
            run_plugin(&spec, env.search, env.timeout)
                .await
                .map_err(|f| to_plugin_error(context, provider, &spec.command, &f, &[]))
        })
        .await?;

    config.auth_info.exec = None;
    config.auth_info.token = credential.token;
    if credential.client_certificate_data.is_some() {
        config.auth_info.client_certificate_data = credential.client_certificate_data;
        config.auth_info.client_key_data = credential.client_key_data;
    }
    Ok(())
}

/// [`prepare_config_with`] using the live policy, search path, cache and default timeout.
pub async fn prepare_config(context: &str, config: &mut Config) -> Result<(), PluginError> {
    if config.auth_info.exec.is_none() {
        return Ok(());
    }
    let policy = super::persistence::load_exec_policies()
        .get(context)
        .copied()
        .unwrap_or_default();
    let search = detect::default_search_dirs();
    prepare_config_with(
        context,
        config,
        &PrepareEnv {
            policy,
            search: &search,
            cache: global_cache(),
            timeout: exec::DEFAULT_TIMEOUT,
        },
    )
    .await
}

/// Build a short-lived client for `context` through the shared credential cache: the path used
/// by the context probe and the fleet poller.
pub async fn build_client(context: &str) -> anyhow::Result<Client> {
    let kubeconfig = load_kubeconfig()?;
    let opts = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..Default::default()
    };
    let mut config = Config::from_custom_kubeconfig(kubeconfig, &opts).await?;
    prepare_config(context, &mut config).await?;
    Ok(Client::try_from(config)?)
}

/// Detect every context's plugin (no spawn), drop cache entries for removed contexts, and
/// emit `CloudAuthDetected`.
pub(super) fn emit_detection(
    seq: &std::sync::atomic::AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
) {
    let infos = detect_infos();
    if let Some(contexts) = infos
        .as_ref()
        .map(|i| i.iter().map(|x| x.context.clone()).collect::<Vec<_>>())
    {
        global_cache().retain_contexts(&contexts);
    }
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::CloudAuthDetected {
            infos: infos.unwrap_or_default(),
        }),
    );
}

fn detect_infos() -> Option<Vec<CloudAuthInfo>> {
    let kubeconfig = load_kubeconfig().ok()?;
    let policies: HashMap<String, ExecPolicy> = super::persistence::load_exec_policies();
    Some(detect::detect_all(
        &kubeconfig,
        &policies,
        &detect::default_search_dirs(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn config_with_exec(command: &str, args: &[&str]) -> Config {
        let yaml = format!(
            r"
apiVersion: v1
kind: Config
clusters:
- name: c
  cluster: {{server: 'https://127.0.0.1:1'}}
contexts:
- {{name: ctx, context: {{cluster: c, user: u}}}}
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: {command}
      args: {args:?}
current-context: ctx
"
        );
        let kc = kube::config::Kubeconfig::from_yaml(&yaml).unwrap();
        futures::executor::block_on(Config::from_custom_kubeconfig(
            kc,
            &KubeConfigOptions::default(),
        ))
        .unwrap()
    }

    fn env<'a>(policy: ExecPolicy, cache: &'a CredentialCache) -> PrepareEnv<'a> {
        PrepareEnv {
            policy,
            search: &[],
            cache,
            timeout: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn missing_plugin_is_typed_and_names_everything() {
        let mut cfg = config_with_exec("gke-gcloud-auth-plugin", &[]);
        let cache = CredentialCache::default();
        let err = prepare_config_with("ctx", &mut cfg, &env(ExecPolicy::AllowKnownOnly, &cache))
            .await
            .unwrap_err();
        assert_eq!(err.kind, PluginErrorKind::PluginMissing);
        assert_eq!(err.provider, CloudProvider::Gke);
        assert_eq!(err.context, "ctx");
        assert!(
            err.hint
                .as_deref()
                .unwrap()
                .contains("gcloud components install")
        );
        let any: anyhow::Error = err.into();
        assert!(plugin_error_of(&any).is_some());
    }

    #[tokio::test]
    async fn unknown_command_is_blocked_by_default_and_allowed_with_allow_all() {
        let mut cfg = config_with_exec("weird-plugin", &[]);
        let cache = CredentialCache::default();
        let err = prepare_config_with("ctx", &mut cfg, &env(ExecPolicy::AllowKnownOnly, &cache))
            .await
            .unwrap_err();
        assert_eq!(err.kind, PluginErrorKind::PluginBlocked);
        assert!(err.message.contains("weird-plugin"));
        // allowAll proceeds to resolution (and here reports it missing rather than blocked).
        let err = prepare_config_with("ctx", &mut cfg, &env(ExecPolicy::AllowAll, &cache))
            .await
            .unwrap_err();
        assert_eq!(err.kind, PluginErrorKind::PluginMissing);
    }

    #[tokio::test]
    async fn static_contexts_are_untouched() {
        let kc = kube::config::Kubeconfig::from_yaml(
            r"
apiVersion: v1
kind: Config
clusters: [{name: c, cluster: {server: 'https://127.0.0.1:1'}}]
contexts: [{name: ctx, context: {cluster: c, user: u}}]
users: [{name: u, user: {token: static-token}}]
",
        )
        .unwrap();
        let mut cfg = Config::from_custom_kubeconfig(
            kc,
            &KubeConfigOptions {
                context: Some("ctx".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cache = CredentialCache::default();
        prepare_config_with("ctx", &mut cfg, &env(ExecPolicy::AllowKnownOnly, &cache))
            .await
            .unwrap();
        assert!(cfg.auth_info.token.is_some());
        assert!(cache.is_empty());
        let _ = Arc::new(());
    }
}

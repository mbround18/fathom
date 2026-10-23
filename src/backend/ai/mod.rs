//! 038-sdk-login-ollama: provider login, secure credential storage, Ollama detection and
//! offline mode.
//!
//! [`AiRuntime`] is the single owner of this state. It is cheap to clone (an `Arc`) so the
//! device-flow polling task can keep updating it after the command that started it has
//! returned (the backend holds its state lock for the whole duration of a command, so
//! long-running work must not run inside one). All outbound traffic goes through
//! [`http::AiHttp`]; all secrets through [`store::CredentialStore`].

pub(crate) mod http;
pub(crate) mod login;
pub(crate) mod migrate;
pub(crate) mod ollama_probe;
pub(crate) mod providers;
pub(crate) mod store;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crossbeam_channel::Sender;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use self::http::{AiHttp, endpoint_is_loopback};
use self::store::CredentialStore;
use crate::backend::agent::AgentClient;
use crate::backend::persistence;
use crate::protocol::v1::{
    AiAssistSettings, BlockedReason, CredentialMigrationOutcome, LoginFlow, LoginPhase, Provider,
    ProviderState, ProviderStatus,
};
use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// Lets spawned tasks (which have no `&BackendState`) emit events.
#[derive(Clone)]
pub(crate) struct Emitter {
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
}

impl Emitter {
    pub(crate) fn new(seq: Arc<AtomicU64>, sender: Sender<BackendEventEnvelope>) -> Self {
        Self { seq, sender }
    }

    pub(crate) fn system(&self, event: SystemEvent) {
        crate::backend::events::emit(
            self.seq.as_ref(),
            &self.sender,
            Route::global(section_keys::system()),
            Correlation::None,
            BackendEvent::System(event),
        );
    }

    pub(crate) fn error(
        &self,
        code: &str,
        message_verbatim: impl Into<String>,
        hint: impl Into<String>,
    ) {
        self.system(SystemEvent::AiError {
            code: code.to_owned(),
            message_verbatim: message_verbatim.into(),
            hint: hint.into(),
        });
    }
}

/// Where each provider is reached. Production defaults; tests point these at stubs.
#[derive(Clone)]
pub(crate) struct Endpoints {
    pub claude_base: String,
    pub openai_base: String,
    pub github_oauth_base: String,
    pub github_api_base: String,
    pub github_chat_url: String,
    pub openai_chat_url: String,
    /// The GitHub OAuth app's public client id (device flow needs no secret).
    pub github_client_id: Option<String>,
    /// One protocol "second"; shrunk by tests.
    pub time_unit: Duration,
}

impl Default for Endpoints {
    fn default() -> Self {
        let client_id = std::env::var("FATHOM_GITHUB_CLIENT_ID")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| option_env!("FATHOM_GITHUB_CLIENT_ID").map(str::to_owned))
            .filter(|v| !v.trim().is_empty());
        Self {
            claude_base: providers::claude::DEFAULT_BASE.to_owned(),
            openai_base: providers::openai::DEFAULT_BASE.to_owned(),
            github_oauth_base: providers::github::DEFAULT_OAUTH_BASE.to_owned(),
            github_api_base: providers::github::DEFAULT_API_BASE.to_owned(),
            github_chat_url: crate::backend::agent::openai_compat::GITHUB_CHAT_URL.to_owned(),
            openai_chat_url: crate::backend::agent::openai_compat::OPENAI_CHAT_URL.to_owned(),
            github_client_id: client_id,
            time_unit: Duration::from_secs(1),
        }
    }
}

pub(crate) struct ActiveFlow {
    pub provider: Provider,
    pub cancel: CancellationToken,
    pub flow: LoginFlow,
}

#[derive(Default)]
pub(crate) struct Session {
    pub settings: AiAssistSettings,
    pub labels: BTreeMap<String, String>,
    /// Hosted providers with a credential in the store (cached; refreshed on demand).
    pub present: HashSet<Provider>,
    pub needs_reauth: HashSet<Provider>,
    pub store_block: Option<BlockedReason>,
    pub last_checked: HashMap<Provider, String>,
    pub flow: Option<ActiveFlow>,
    pub migration_notice: Option<CredentialMigrationOutcome>,
}

struct Inner {
    store: Arc<CredentialStore>,
    http: Arc<AiHttp>,
    endpoints: Endpoints,
    config_path: Option<PathBuf>,
    legacy_path: Option<PathBuf>,
    session: Mutex<Session>,
}

#[derive(Clone)]
pub(crate) struct AiRuntime {
    inner: Arc<Inner>,
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl AiRuntime {
    /// Production wiring: OS keyring, real endpoints, `~/.fathom/*` paths.
    pub(crate) fn production() -> Self {
        Self::new(
            Arc::new(CredentialStore::system()),
            Arc::new(AiHttp::new()),
            Endpoints::default(),
            persistence::main_config_path().ok(),
            persistence::credentials_config_path().ok(),
        )
    }

    pub(crate) fn new(
        store: Arc<CredentialStore>,
        http: Arc<AiHttp>,
        endpoints: Endpoints,
        config_path: Option<PathBuf>,
        legacy_path: Option<PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                http,
                endpoints,
                config_path,
                legacy_path,
                session: Mutex::new(Session::default()),
            }),
        }
    }

    pub(crate) fn http(&self) -> &Arc<AiHttp> {
        &self.inner.http
    }

    pub(crate) fn store(&self) -> &Arc<CredentialStore> {
        &self.inner.store
    }

    pub(crate) fn endpoints(&self) -> &Endpoints {
        &self.inner.endpoints
    }

    pub(crate) fn session(&self) -> MutexGuard<'_, Session> {
        self.inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn settings(&self) -> AiAssistSettings {
        self.session().settings.clone()
    }

    /// Persist settings (best effort; the in-memory value stays authoritative).
    pub(crate) fn persist_settings(&self) {
        let Some(path) = &self.inner.config_path else {
            return;
        };
        let settings = self.settings();
        if let Err(e) = persistence::save_ai_settings_at(path, &settings) {
            tracing::warn!("Could not persist AI-assist settings: {e}");
        }
    }

    pub(crate) fn persist_label(&self, provider: Provider, label: Option<&str>) {
        let Some(path) = &self.inner.config_path else {
            return;
        };
        if let Err(e) = persistence::save_ai_account_label_at(path, provider, label) {
            tracing::warn!("Could not persist AI-assist account label: {e}");
        }
    }

    // ------------------------------------------------------------------ bootstrap

    /// Startup: load settings, apply offline + file opt-in, run the legacy migration, and
    /// learn which credentials exist.
    pub(crate) async fn bootstrap(&self, em: &Emitter) {
        self.reload_settings();
        {
            let s = self.session();
            self.inner.http.set_offline(s.settings.offline_mode);
            self.inner.store.set_file_optin(s.settings.file_store_optin);
        }
        if let (Some(legacy), Some(config)) = (&self.inner.legacy_path, &self.inner.config_path) {
            let store = Arc::clone(&self.inner.store);
            let (legacy, config) = (legacy.clone(), config.clone());
            let outcome =
                tokio::task::spawn_blocking(move || migrate::migrate(&store, &legacy, &config))
                    .await
                    .unwrap_or_else(|e| CredentialMigrationOutcome::Failed {
                        message: format!("migration task failed: {e}"),
                    });
            if outcome != CredentialMigrationOutcome::NothingToMigrate {
                match &outcome {
                    CredentialMigrationOutcome::Failed { message } => {
                        tracing::warn!("AI credential migration failed: {message}");
                    }
                    _ => tracing::info!("AI credential migration completed"),
                }
                self.session().migration_notice = Some(outcome);
            }
            self.reload_settings();
        }
        self.refresh_presence().await;
        self.emit_status(em);
    }

    fn reload_settings(&self) {
        let Some(path) = &self.inner.config_path else {
            return;
        };
        let settings = persistence::load_ai_settings_at(path);
        let labels = persistence::load_ai_account_labels_at(path);
        let mut s = self.session();
        s.settings = settings;
        s.labels = labels;
    }

    /// Ask the store which hosted providers have a credential. A locked/unavailable store
    /// becomes a `Blocked` state rather than an error.
    pub(crate) async fn refresh_presence(&self) {
        let mut present = HashSet::new();
        let mut block = None;
        for provider in Provider::ALL.into_iter().filter(|p| p.is_hosted()) {
            match self.inner.store.get_async(provider).await {
                Ok(Some(_)) => {
                    present.insert(provider);
                }
                Ok(None) => {}
                Err(e) => {
                    block = e.blocked_reason();
                    break;
                }
            }
        }
        let now = now_rfc3339();
        let mut s = self.session();
        for p in &present {
            s.last_checked.insert(*p, now.clone());
        }
        s.present = present;
        s.store_block = block;
    }

    // ------------------------------------------------------------------ status

    /// Offline mode forces Ollama; it never falls back to a hosted provider.
    pub(crate) fn effective_provider(&self) -> Option<Provider> {
        let s = self.session();
        if s.settings.offline_mode {
            Some(Provider::Ollama)
        } else {
            s.settings.active_provider
        }
    }

    pub(crate) fn status_event(&self) -> SystemEvent {
        let active = self.effective_provider();
        let s = self.session();
        let statuses = Provider::ALL
            .into_iter()
            .map(|provider| {
                let connecting = s.flow.as_ref().is_some_and(|f| {
                    f.provider == provider
                        && matches!(f.flow.phase, LoginPhase::Pending | LoginPhase::Polling)
                });
                let state = if provider == Provider::Ollama {
                    if s.settings.ollama_model.is_some() {
                        ProviderState::Connected
                    } else {
                        ProviderState::NotConfigured
                    }
                } else if s.settings.offline_mode {
                    ProviderState::Blocked {
                        reason: BlockedReason::Offline,
                    }
                } else if let Some(reason) = &s.store_block {
                    ProviderState::Blocked {
                        reason: reason.clone(),
                    }
                } else if s.needs_reauth.contains(&provider) {
                    ProviderState::NeedsReauth
                } else if connecting {
                    ProviderState::Connecting
                } else if s.present.contains(&provider) {
                    ProviderState::Connected
                } else {
                    ProviderState::NotConfigured
                };
                ProviderStatus {
                    provider,
                    state,
                    account_label: s.labels.get(provider.as_str()).cloned(),
                    last_checked_at: s.last_checked.get(&provider).cloned(),
                }
            })
            .collect();
        SystemEvent::AiProviderStatus {
            statuses,
            active,
            offline: s.settings.offline_mode,
            ollama_model: s.settings.ollama_model.clone(),
        }
    }

    pub(crate) fn emit_status(&self, em: &Emitter) {
        em.system(self.status_event());
    }

    pub(crate) async fn get_status(&self, em: &Emitter) {
        self.refresh_presence().await;
        let notice = self.session().migration_notice.take();
        if let Some(outcome) = notice {
            em.system(SystemEvent::AiCredentialMigration { outcome });
        }
        self.emit_status(em);
    }

    // ------------------------------------------------------------------ settings commands

    pub(crate) async fn set_active_provider(&self, provider: Option<Provider>, em: &Emitter) {
        if let Some(p) = provider
            && p.is_hosted()
            && !self.session().present.contains(&p)
        {
            em.error(
                "provider_not_connected",
                format!("{} is not signed in.", p.as_str()),
                "Sign in to this provider first.",
            );
            return;
        }
        self.session().settings.active_provider = provider;
        self.persist_settings();
        self.emit_status(em);
    }

    pub(crate) fn set_offline_mode(&self, enabled: bool, em: &Emitter) {
        self.session().settings.offline_mode = enabled;
        self.inner.http.set_offline(enabled);
        if enabled {
            self.cancel_flow_silently();
        }
        self.persist_settings();
        self.emit_status(em);
    }

    pub(crate) async fn enable_file_store(&self, confirm: bool, em: &Emitter) {
        if !confirm {
            em.error(
                "confirmation_required",
                "The file credential store stores secrets unencrypted in ~/.fathom (mode 0600).",
                "Confirm to opt in.",
            );
            return;
        }
        self.session().settings.file_store_optin = true;
        self.inner.store.set_file_optin(true);
        self.persist_settings();
        self.refresh_presence().await;
        self.emit_status(em);
    }

    // ------------------------------------------------------------------ ollama commands

    pub(crate) async fn ollama_probe(&self, endpoint: Option<String>, em: &Emitter) {
        let settings = self.settings();
        let endpoint = endpoint.unwrap_or_else(|| settings.ollama_endpoint.clone());
        if !endpoint_is_loopback(&endpoint) && !settings.allow_lan_ollama {
            em.error(
                "lan_confirmation_required",
                format!("{endpoint} is not a loopback address."),
                "Confirm LAN use for this endpoint before probing it.",
            );
            return;
        }
        let state = ollama_probe::probe(&self.inner.http, &endpoint).await;
        let using_saved = endpoint == settings.ollama_endpoint;
        let mut selected = settings.ollama_model.clone();
        if using_saved
            && state.reachable
            && state.error.is_none()
            && let Some(model) = selected.clone()
            && !state.models.iter().any(|m| m.name == model)
        {
            self.clear_model(&model);
            selected = None;
            em.error(
                "ollama_model_missing",
                format!("Ollama model \"{model}\" is no longer installed at {endpoint}."),
                "Pick another model.",
            );
            self.emit_status(em);
        }
        em.system(SystemEvent::AiOllamaState {
            state,
            selected_model: selected,
        });
    }

    fn clear_model(&self, model: &str) {
        {
            let mut s = self.session();
            if s.settings.ollama_model.as_deref() == Some(model) {
                s.settings.ollama_model = None;
            }
        }
        self.persist_settings();
    }

    pub(crate) fn ollama_select_model(&self, model: String, em: &Emitter) {
        if model.trim().is_empty() {
            em.error(
                "invalid_model",
                "Model name is empty.",
                "Pick a model from the list.",
            );
            return;
        }
        {
            let mut s = self.session();
            s.settings.ollama_model = Some(model);
            if s.settings.active_provider.is_none() {
                s.settings.active_provider = Some(Provider::Ollama);
            }
        }
        self.persist_settings();
        self.emit_status(em);
    }

    pub(crate) async fn set_ollama_endpoint(
        &self,
        endpoint: String,
        allow_lan: bool,
        em: &Emitter,
    ) {
        let endpoint = endpoint.trim().to_owned();
        let valid = reqwest::Url::parse(&endpoint)
            .map(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .unwrap_or(false);
        if !valid {
            em.error(
                "invalid_endpoint",
                format!("\"{endpoint}\" is not a valid http(s) URL."),
                "Use a URL like http://127.0.0.1:11434.",
            );
            return;
        }
        if !endpoint_is_loopback(&endpoint) && !allow_lan {
            em.error(
                "lan_confirmation_required",
                format!("{endpoint} is not a loopback address; prompts and cluster context would leave this machine."),
                "Confirm that you trust this host to use it.",
            );
            return;
        }
        {
            let mut s = self.session();
            if s.settings.ollama_endpoint != endpoint {
                s.settings.ollama_model = None;
            }
            s.settings.ollama_endpoint = endpoint;
            s.settings.allow_lan_ollama = allow_lan;
        }
        self.persist_settings();
        self.emit_status(em);
        self.ollama_probe(None, em).await;
    }

    // ------------------------------------------------------------------ assistant turns

    /// Build the client for the next assistant turn from the effective provider.
    pub(crate) async fn turn_client(&self) -> anyhow::Result<Box<dyn AgentClient>> {
        use crate::backend::agent::{
            anthropic::AnthropicClient, ollama::OllamaClient, openai_compat,
        };
        let settings = self.settings();
        let Some(provider) = self.effective_provider() else {
            anyhow::bail!(
                "The assistant is not configured yet. Open Settings -> AI Assist and sign in to a \
                 provider or pick a local Ollama model."
            );
        };
        let http = Arc::clone(&self.inner.http);
        if provider == Provider::Ollama {
            let Some(model) = settings.ollama_model.clone() else {
                if settings.offline_mode {
                    anyhow::bail!(
                        "Offline mode is on and no local Ollama model is selected. Pick a model in \
                         Settings -> AI Assist."
                    );
                }
                anyhow::bail!("No Ollama model is selected. Pick one in Settings -> AI Assist.");
            };
            if !endpoint_is_loopback(&settings.ollama_endpoint) && !settings.allow_lan_ollama {
                anyhow::bail!(
                    "The Ollama endpoint {} is not loopback and LAN use was not confirmed.",
                    settings.ollama_endpoint
                );
            }
            return Ok(Box::new(OllamaClient::new(
                http,
                settings.ollama_endpoint,
                model,
            )));
        }
        if self.session().needs_reauth.contains(&provider) {
            anyhow::bail!(
                "{} needs you to sign in again (the stored credential was rejected). Open Settings -> AI Assist.",
                provider.as_str()
            );
        }
        let secret: SecretString = match self.inner.store.get_async(provider).await {
            Ok(Some(s)) => s,
            Ok(None) => anyhow::bail!(
                "{} is not signed in. Open Settings -> AI Assist.",
                provider.as_str()
            ),
            Err(e) => anyhow::bail!("{} {}", e.message(), e.hint()),
        };
        let ep = &self.inner.endpoints;
        Ok(match provider {
            Provider::Claude => {
                use secrecy::ExposeSecret;
                Box::new(AnthropicClient::new_with_url(
                    http,
                    secret.expose_secret().to_owned(),
                    format!("{}/v1/messages", ep.claude_base.trim_end_matches('/')),
                ))
            }
            Provider::Openai => Box::new(openai_compat::OpenAiCompatClient::new(
                http,
                Provider::Openai,
                ep.openai_chat_url.clone(),
                secret,
                openai_compat::OPENAI_MODEL.to_owned(),
            )),
            Provider::Github => Box::new(openai_compat::OpenAiCompatClient::new(
                http,
                Provider::Github,
                ep.github_chat_url.clone(),
                secret,
                openai_compat::GITHUB_MODEL.to_owned(),
            )),
            Provider::Ollama => unreachable!("handled above"),
        })
    }

    /// T017: an assistant turn was rejected with 401 -> the provider needs re-auth.
    pub(crate) fn mark_needs_reauth(&self, provider: Provider, em: &Emitter) {
        self.session().needs_reauth.insert(provider);
        self.emit_status(em);
    }

    /// The selected Ollama model vanished (404 from `/api/chat`): clear it, re-probe, tell
    /// the operator.
    pub(crate) async fn model_missing(&self, model: &str, em: &Emitter) {
        self.clear_model(model);
        em.error(
            "ollama_model_missing",
            format!("Ollama model \"{model}\" was not found."),
            "Pick another model in Settings -> AI Assist.",
        );
        self.emit_status(em);
        self.ollama_probe(None, em).await;
    }

    /// Non-secret cancel used by offline toggling.
    fn cancel_flow_silently(&self) {
        if let Some(f) = self.session().flow.take() {
            f.cancel.cancel();
        }
    }
}

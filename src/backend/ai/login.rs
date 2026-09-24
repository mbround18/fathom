//! Login, key submission and sign-out (038 US1/US2). Secrets only ever pass through
//! `SecretString`; nothing here logs or emits one.

use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use super::providers::claude::ClaudeAdapter;
use super::providers::github::{DeviceCode, DeviceOutcome, GithubAdapter};
use super::providers::openai::OpenAiAdapter;
use super::providers::{ProviderAdapter, ProviderError};
use super::store::StoreError;
use super::{ActiveFlow, AiRuntime, Emitter, now_rfc3339};
use crate::protocol::SystemEvent;
use crate::protocol::v1::{ApiKeyInput, LoginFlow, LoginFlowKind, LoginPhase, Provider};

const GITHUB_REVOKE_URL: &str = "github.com/settings/applications";

fn emit_provider_error(em: &Emitter, err: &ProviderError) {
    em.error(err.code(), err.message_verbatim(), err.hint());
}

fn emit_store_error(em: &Emitter, err: &StoreError) {
    em.error(err.code(), err.message(), err.hint());
}

impl AiRuntime {
    fn api_key_adapter(&self, provider: Provider) -> Option<Box<dyn ProviderAdapter>> {
        let ep = self.endpoints();
        match provider {
            Provider::Claude => Some(Box::new(ClaudeAdapter::new(ep.claude_base.clone()))),
            Provider::Openai => Some(Box::new(OpenAiAdapter::new(ep.openai_base.clone()))),
            _ => None,
        }
    }

    fn github_adapter(&self) -> Option<GithubAdapter> {
        let ep = self.endpoints();
        let client_id = ep.github_client_id.clone()?;
        Some(
            GithubAdapter::new(
                client_id,
                ep.github_oauth_base.clone(),
                ep.github_api_base.clone(),
            )
            .with_time_unit(ep.time_unit),
        )
    }

    fn set_flow(&self, flow: LoginFlow, cancel: Option<CancellationToken>, em: &Emitter) {
        {
            let mut s = self.session();
            if let Some(cancel) = cancel {
                s.flow = Some(ActiveFlow {
                    provider: flow.provider,
                    cancel,
                    flow: flow.clone(),
                });
            } else if let Some(active) = s.flow.as_mut()
                && active.provider == flow.provider
            {
                active.flow = flow.clone();
            }
        }
        em.system(SystemEvent::AiLoginFlowUpdate { flow });
    }

    fn end_flow(&self, provider: Provider) {
        let mut s = self.session();
        if s.flow.as_ref().is_some_and(|f| f.provider == provider) {
            s.flow = None;
        }
    }

    fn blank_flow(provider: Provider, kind: LoginFlowKind, phase: LoginPhase) -> LoginFlow {
        LoginFlow {
            provider,
            kind,
            user_code: None,
            verification_uri: None,
            expires_at: None,
            interval_s: None,
            phase,
        }
    }

    pub(crate) async fn login_start(&self, provider: Provider, em: &Emitter) {
        if !provider.is_hosted() {
            em.error(
                "no_login_needed",
                "Ollama does not use a login.",
                "Pick a model in the Ollama section instead.",
            );
            return;
        }
        if self.http().is_offline() {
            emit_provider_error(
                em,
                &ProviderError::Offline(
                    "Offline mode is on: hosted providers are unavailable.".to_owned(),
                ),
            );
            return;
        }
        if self.session().flow.is_some() {
            em.error(
                "login_in_progress",
                "Another sign-in is already in progress.",
                "Finish or cancel it first.",
            );
            return;
        }
        if provider != Provider::Github {
            let flow = Self::blank_flow(provider, LoginFlowKind::ApiKey, LoginPhase::Pending);
            self.set_flow(flow, Some(CancellationToken::new()), em);
            self.emit_status(em);
            return;
        }
        let Some(adapter) = self.github_adapter() else {
            em.error(
                "github_client_id_missing",
                "This build has no GitHub OAuth client id configured.",
                "Set FATHOM_GITHUB_CLIENT_ID (a public GitHub OAuth app client id with device flow enabled) and restart.",
            );
            return;
        };
        let device = match adapter.start(self.http()).await {
            Ok(d) => d,
            Err(e) => {
                emit_provider_error(em, &e);
                return;
            }
        };
        let expires_at = (k8s_openapi::chrono::Utc::now()
            + k8s_openapi::chrono::Duration::seconds(device.expires_in_s as i64))
        .to_rfc3339();
        let base = LoginFlow {
            provider,
            kind: LoginFlowKind::DeviceCode,
            user_code: Some(device.user_code.clone()),
            verification_uri: Some(device.verification_uri.clone()),
            expires_at: Some(expires_at),
            interval_s: Some(device.interval_s as u32),
            phase: LoginPhase::Pending,
        };
        let cancel = CancellationToken::new();
        // Re-check the single-flow guard now that the request has completed.
        if self.session().flow.is_some() {
            em.error(
                "login_in_progress",
                "Another sign-in is already in progress.",
                "Finish or cancel it first.",
            );
            return;
        }
        self.set_flow(base.clone(), Some(cancel.clone()), em);
        self.emit_status(em);
        let rt = self.clone();
        let em = em.clone();
        tokio::spawn(async move {
            rt.run_device_flow(adapter, device, base, cancel, em).await;
        });
    }

    async fn run_device_flow(
        &self,
        adapter: GithubAdapter,
        device: DeviceCode,
        base: LoginFlow,
        cancel: CancellationToken,
        em: Emitter,
    ) {
        let outcome = {
            let (rt, em2, base2) = (self.clone(), em.clone(), base.clone());
            let on_phase = move |phase: LoginPhase, interval_s: u64| {
                let mut flow = base2.clone();
                flow.phase = phase;
                flow.interval_s = Some(interval_s as u32);
                rt.set_flow(flow, None, &em2);
            };
            adapter
                .drive(self.http(), &device, &cancel, &on_phase)
                .await
        };
        let provider = base.provider;
        let mut flow = base;
        match outcome {
            DeviceOutcome::Token(token) => {
                self.finish_login(provider, &adapter, token, LoginFlowKind::DeviceCode, &em)
                    .await;
                return;
            }
            DeviceOutcome::Denied => flow.phase = LoginPhase::Denied,
            DeviceOutcome::Expired => flow.phase = LoginPhase::Expired,
            DeviceOutcome::Failed(message) => {
                em.error(
                    "github_login_failed",
                    message.clone(),
                    "Try signing in again.",
                );
                flow.phase = LoginPhase::Failed { message };
            }
            DeviceOutcome::Cancelled => {
                flow.phase = LoginPhase::Failed {
                    message: "Sign-in cancelled.".to_owned(),
                };
            }
        }
        self.end_flow(provider);
        em.system(SystemEvent::AiLoginFlowUpdate { flow });
        self.emit_status(&em);
    }

    /// Validate the credential, store it verified, and update every piece of state.
    async fn finish_login(
        &self,
        provider: Provider,
        adapter: &dyn ProviderAdapter,
        secret: SecretString,
        kind: LoginFlowKind,
        em: &Emitter,
    ) -> bool {
        let fail = |phase_msg: String| {
            let mut flow =
                Self::blank_flow(provider, kind, LoginPhase::Failed { message: phase_msg });
            flow.provider = provider;
            flow
        };
        let account = match adapter.validate(self.http(), &secret).await {
            Ok(a) => a,
            Err(e) => {
                emit_provider_error(em, &e);
                self.end_flow(provider);
                em.system(SystemEvent::AiLoginFlowUpdate {
                    flow: fail(e.message_verbatim()),
                });
                self.emit_status(em);
                return false;
            }
        };
        if let Err(e) = self.store().save_verified_async(provider, secret).await {
            emit_store_error(em, &e);
            if let Some(reason) = e.blocked_reason() {
                self.session().store_block = Some(reason);
            }
            self.end_flow(provider);
            em.system(SystemEvent::AiLoginFlowUpdate {
                flow: fail(e.message().to_owned()),
            });
            self.emit_status(em);
            return false;
        }
        {
            let mut s = self.session();
            s.present.insert(provider);
            s.needs_reauth.remove(&provider);
            s.store_block = None;
            s.last_checked.insert(provider, now_rfc3339());
            match &account.label {
                Some(label) => {
                    s.labels.insert(provider.as_str().to_owned(), label.clone());
                }
                None => {
                    s.labels.remove(provider.as_str());
                }
            }
            if s.settings.active_provider.is_none() {
                s.settings.active_provider = Some(provider);
            }
        }
        self.persist_label(provider, account.label.as_deref());
        self.persist_settings();
        self.end_flow(provider);
        em.system(SystemEvent::AiLoginFlowUpdate {
            flow: Self::blank_flow(provider, kind, LoginPhase::Succeeded),
        });
        self.emit_status(em);
        true
    }

    pub(crate) async fn submit_api_key(&self, provider: Provider, key: ApiKeyInput, em: &Emitter) {
        let Some(adapter) = self.api_key_adapter(provider) else {
            em.error(
                "unsupported_login",
                format!("{} does not use an API key.", provider.as_str()),
                "Use the sign-in button for this provider.",
            );
            return;
        };
        if self.http().is_offline() {
            emit_provider_error(
                em,
                &ProviderError::Offline(
                    "Offline mode is on: hosted providers are unavailable.".to_owned(),
                ),
            );
            return;
        }
        let trimmed = key.0.trim().to_owned();
        drop(key);
        if trimmed.is_empty() {
            em.error(
                "empty_key",
                "The API key is empty.",
                "Paste your API key and try again.",
            );
            return;
        }
        let secret = SecretString::from(trimmed);
        self.session().flow.take();
        self.finish_login(
            provider,
            adapter.as_ref(),
            secret,
            LoginFlowKind::ApiKey,
            em,
        )
        .await;
    }

    pub(crate) fn login_cancel(&self, em: &Emitter) {
        let Some(active) = self.session().flow.take() else {
            return;
        };
        active.cancel.cancel();
        let mut flow = active.flow;
        flow.phase = LoginPhase::Failed {
            message: "Sign-in cancelled.".to_owned(),
        };
        em.system(SystemEvent::AiLoginFlowUpdate { flow });
        self.emit_status(em);
    }

    pub(crate) async fn sign_out(&self, provider: Provider, em: &Emitter) {
        if !provider.is_hosted() {
            em.error(
                "no_login_needed",
                "Ollama has no account to sign out of.",
                "Clear the model instead.",
            );
            return;
        }
        if let Err(e) = self.store().delete_async(provider).await {
            emit_store_error(em, &e);
            return;
        }
        {
            let mut s = self.session();
            s.present.remove(&provider);
            s.needs_reauth.remove(&provider);
            s.labels.remove(provider.as_str());
            s.last_checked.remove(&provider);
            if s.settings.active_provider == Some(provider) {
                s.settings.active_provider = None;
            }
        }
        self.persist_label(provider, None);
        self.persist_settings();
        if provider == Provider::Github {
            // A device-flow token cannot be revoked without the app's client secret, which
            // a desktop app must not ship. Be explicit rather than pretend it was revoked.
            em.error(
                "github_revoke_manual",
                "The token was removed from this device but could not be revoked at GitHub.",
                format!("Revoke it at {GITHUB_REVOKE_URL} if you want it invalidated."),
            );
        }
        self.emit_status(em);
    }
}

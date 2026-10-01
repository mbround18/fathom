//! Per-issuer OIDC sessions (033 US1, research R6).
//!
//! One tokio task per signed-in issuer sleeps until `exp - lead`, redeems the (rotating) refresh
//! token, validates the new ID token and swaps it into an [`ArcSwapOption`]. No polling. The
//! kube bearer layer (US2) reads the current ID token through [`OidcSessions::token_source`].
//! Failure ends the session with [`SystemEvent::OidcSessionEnded`]; nothing here ever falls back
//! to another credential.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::flow::{self, BrowserResult, Discovery, FlowError, Opener, TokenResponse};
use super::store::{CredentialStore, Secret, SecretKind};
use super::validate::{ValidationError, Validator, VerifiedClaims};
use crate::protocol::v1::{
    DraftSignInResult, OidcIdentity, OidcIdentityState, OidcIssuerConfig, OidcSignInMethod,
    OidcSignInStage, SystemEvent,
};

/// Receives every event the sessions emit (the backend wires this to the event channel).
pub type EventSink = Arc<dyn Fn(SystemEvent) + Send + Sync>;

/// Yields the current ID token for the kube bearer layer, or the reason there is none.
pub type TokenSource = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Refresh this long before `exp` (research R6).
pub const DEFAULT_REFRESH_LEAD: Duration = Duration::from_secs(20);

/// The key 045's draft sign-in is tracked under: its in-flight entry, its `Live` slot and the
/// `issuer_id` on its progress events. The protocol owns the value because the UI matches on it;
/// see [`crate::protocol::v1::OIDC_DRAFT_SESSION_KEY`] for why it cannot collide with a saved id.
pub use crate::protocol::v1::OIDC_DRAFT_SESSION_KEY as DRAFT_SESSION_KEY;

const TRANSIENT_RETRY: Duration = Duration::from_secs(5);

struct Live {
    id_token: Secret,
    identity: OidcIdentity,
}

#[derive(Default)]
struct Slot {
    live: ArcSwapOption<Live>,
}

/// What a finished sign-in becomes. The two differ only at the very end of the flow, which is
/// why 045's guided path needs no second flow of its own (research R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// A persisted session: the refresh token is stored and a refresher is started.
    Session,
    /// A draft's evidence: in-memory only, no credential written (045 R2).
    Draft,
}

struct Inner {
    store: Arc<CredentialStore>,
    sink: RwLock<EventSink>,
    opener: Opener,
    refresh_lead: Duration,
    browser_timeout: Duration,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    sign_ins: Mutex<HashMap<String, (CancellationToken, JoinHandle<()>)>>,
    refreshers: Mutex<HashMap<String, JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct OidcSessions {
    inner: Arc<Inner>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl OidcSessions {
    pub fn new(store: Arc<CredentialStore>, sink: EventSink) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                sink: RwLock::new(sink),
                opener: flow::system_opener(),
                refresh_lead: DEFAULT_REFRESH_LEAD,
                browser_timeout: flow::BROWSER_TIMEOUT,
                slots: Mutex::default(),
                sign_ins: Mutex::default(),
                refreshers: Mutex::default(),
            }),
        }
    }

    fn configure(mut self, f: impl FnOnce(&mut Inner)) -> Self {
        // Builder-style: only valid before any task is spawned.
        f(Arc::get_mut(&mut self.inner).expect("configure OidcSessions before use"));
        self
    }

    #[must_use]
    pub fn with_opener(self, opener: Opener) -> Self {
        self.configure(|i| i.opener = opener)
    }

    #[must_use]
    pub fn with_refresh_lead(self, lead: Duration) -> Self {
        self.configure(|i| i.refresh_lead = lead)
    }

    #[must_use]
    pub fn with_browser_timeout(self, timeout: Duration) -> Self {
        self.configure(|i| i.browser_timeout = timeout)
    }

    /// Re-point event delivery (the backend binds its channel after construction).
    pub fn set_sink(&self, sink: EventSink) {
        *self.inner.sink.write().expect("sink") = sink;
    }

    fn slot(&self, issuer_id: &str) -> Arc<Slot> {
        self.inner
            .slots
            .lock()
            .expect("slots")
            .entry(issuer_id.to_owned())
            .or_default()
            .clone()
    }

    /// The credential store (admin API key access for the IdP directory, US3).
    pub fn credentials(&self) -> Arc<CredentialStore> {
        self.inner.store.clone()
    }

    /// Current identity per signed-in issuer. The draft slot is deliberately absent: a draft is
    /// evidence of one sign-in, not an issuer the UI lists (045 FR-007).
    pub fn identities(&self) -> Vec<OidcIdentity> {
        let slots = self.inner.slots.lock().expect("slots");
        let mut out: Vec<OidcIdentity> = slots
            .iter()
            .filter(|(id, _)| id.as_str() != DRAFT_SESSION_KEY)
            .filter_map(|(_, s)| s.live.load_full().map(|l| l.identity.clone()))
            .collect();
        out.sort_by(|a, b| a.issuer_id.cmp(&b.issuer_id));
        out
    }

    /// The token provider for an issuer: the ID token while signed in, otherwise the reason.
    pub fn token_source(&self, issuer_id: &str) -> TokenSource {
        let slot = self.slot(issuer_id);
        let issuer = issuer_id.to_owned();
        Arc::new(move || match slot.live.load_full() {
            Some(live) => Ok(live.id_token.expose().to_owned()),
            None => Err(format!(
                "OIDC session for issuer {issuer:?} ended or was never started; sign in again (kubeconfig credentials are not used for OIDC contexts)"
            )),
        })
    }

    /// Start (or restart) a sign-in in the background. Progress and the outcome arrive as events.
    pub fn sign_in(&self, cfg: OidcIssuerConfig, method: OidcSignInMethod) {
        self.start_sign_in(cfg, method, Outcome::Session);
    }

    /// Run a complete sign-in against an unsaved draft (045 US2, FR-004).
    ///
    /// Reaches the same flow the saved path does -- the draft is carried inline, which is why
    /// `sign_in` taking its config by value is enough -- but nothing is persisted: no refresh
    /// token reaches the credential store and the tokens live in the [`DRAFT_SESSION_KEY`] slot
    /// until `OidcSaveDraft` adopts them. Abandoning the guided path therefore leaves no
    /// credential for an issuer that was never saved (FR-007).
    pub fn test_sign_in(&self, cfg: OidcIssuerConfig, method: OidcSignInMethod) {
        let mut cfg = cfg;
        // The flow keys everything on `cfg.id`, and the draft's id may be one a saved, signed-in
        // issuer already holds: reusing it would cancel that issuer's sign-in and overwrite the
        // identity it published.
        cfg.id = DRAFT_SESSION_KEY.to_owned();
        self.start_sign_in(cfg, method, Outcome::Draft);
    }

    fn start_sign_in(&self, cfg: OidcIssuerConfig, method: OidcSignInMethod, outcome: Outcome) {
        self.cancel_sign_in(&cfg.id);
        let cancel = CancellationToken::new();
        let this = self.clone();
        let id = cfg.id.clone();
        let token = cancel.clone();
        let handle = tokio::spawn(async move {
            this.run_sign_in(cfg, method, token, outcome).await;
        });
        self.inner
            .sign_ins
            .lock()
            .expect("sign_ins")
            .insert(id, (cancel, handle));
    }

    pub fn cancel_sign_in(&self, issuer_id: &str) {
        if let Some((cancel, handle)) = self
            .inner
            .sign_ins
            .lock()
            .expect("sign_ins")
            .remove(issuer_id)
        {
            cancel.cancel();
            // The task reports `cancelled` itself; abort only guarantees it stops promptly if it
            // is parked somewhere that does not observe the token.
            drop(handle);
        }
    }

    /// Delete tokens and end the session. Optionally opens the IdP's end-session URL.
    pub async fn sign_out(&self, cfg: &OidcIssuerConfig, end_session: bool) {
        self.cancel_sign_in(&cfg.id);
        self.stop_refresher(&cfg.id);
        self.slot(&cfg.id).live.store(None);
        self.inner.store.clear_issuer(&cfg.id, false);
        self.emit(SystemEvent::OidcIdentityUpdated {
            issuer_id: cfg.id.clone(),
            identity: None,
        });
        if end_session
            && let Ok(http) = flow::http_client(cfg)
            && let Ok(disc) = flow::discover(&http, &cfg.issuer_url).await
            && let Some(url) = disc.end_session_endpoint
            && let Err(e) = (self.inner.opener)(&url)
        {
            self.emit(SystemEvent::OidcError {
                issuer_id: cfg.id.clone(),
                stage: "logout".into(),
                message: e,
            });
        }
    }

    /// Forget everything about an issuer (config deleted).
    pub fn forget(&self, issuer_id: &str) {
        self.cancel_sign_in(issuer_id);
        self.stop_refresher(issuer_id);
        self.inner.slots.lock().expect("slots").remove(issuer_id);
        self.inner.store.clear_issuer(issuer_id, true);
    }

    /// Resume a previous session from the stored refresh token (app start). Silent when there is
    /// none; a rejected token ends the session with the verbatim reason.
    pub async fn restore(&self, cfg: &OidcIssuerConfig) {
        if self.slot(&cfg.id).live.load().is_some() {
            return;
        }
        let Some(refresh) = self.inner.store.get(SecretKind::RefreshToken, &cfg.id) else {
            return;
        };
        let Ok(http) = flow::http_client(cfg) else {
            return;
        };
        let Ok(disc) = flow::discover(&http, &cfg.issuer_url).await else {
            return;
        };
        let Ok(validator) = self.validator(&http, &disc, cfg).await else {
            return;
        };
        match flow::refresh(&http, &disc, cfg, refresh.expose()).await {
            Ok(tokens) => match validator.validate(&tokens.id_token, None).await {
                Ok(claims) => {
                    self.install(
                        cfg,
                        &http,
                        &disc,
                        Arc::new(validator),
                        tokens,
                        claims,
                        refresh.expose(),
                    );
                }
                Err(e) => self.end_session(&cfg.id, &e.0),
            },
            Err(e) if e.code == "transport" => {} // offline at start: stay signed out quietly
            Err(e) => self.end_session(&cfg.id, &e.message),
        }
    }

    // ---- internals -----------------------------------------------------------------------

    fn emit(&self, event: SystemEvent) {
        let sink = self.inner.sink.read().expect("sink").clone();
        sink(event);
    }

    fn error(&self, issuer_id: &str, stage: &str, message: impl Into<String>) {
        self.emit(SystemEvent::OidcError {
            issuer_id: issuer_id.to_owned(),
            stage: stage.to_owned(),
            message: message.into(),
        });
    }

    fn progress(&self, issuer_id: &str, stage: OidcSignInStage) {
        self.emit(SystemEvent::OidcSignInProgress {
            issuer_id: issuer_id.to_owned(),
            stage,
        });
    }

    fn stop_refresher(&self, issuer_id: &str) {
        if let Some(h) = self
            .inner
            .refreshers
            .lock()
            .expect("refreshers")
            .remove(issuer_id)
        {
            h.abort();
        }
    }

    async fn validator(
        &self,
        http: &reqwest::Client,
        disc: &Discovery,
        cfg: &OidcIssuerConfig,
    ) -> Result<Validator, ValidationError> {
        Validator::new(
            http.clone(),
            disc.jwks_uri.clone(),
            &disc.issuer,
            &cfg.client_id,
            &cfg.username_claim,
            &cfg.groups_claim,
        )
        .await
    }

    async fn run_sign_in(
        &self,
        cfg: OidcIssuerConfig,
        method: OidcSignInMethod,
        cancel: CancellationToken,
        outcome: Outcome,
    ) {
        let id = cfg.id.clone();
        let http = match flow::http_client(&cfg) {
            Ok(h) => h,
            Err(e) => return self.error(&id, "config", e),
        };
        let disc = match flow::discover(&http, &cfg.issuer_url).await {
            Ok(d) => d,
            Err(e) => return self.error(&id, "discovery", e.to_string()),
        };

        let mut method = method;
        let mut browser: Option<BrowserResult> = None;
        let mut device: Option<TokenResponse> = None;
        if method == OidcSignInMethod::Browser {
            self.progress(&id, OidcSignInStage::Awaiting);
            match flow::browser_flow(
                &http,
                &disc,
                &cfg,
                &self.inner.opener,
                &cancel,
                self.inner.browser_timeout,
            )
            .await
            {
                Ok(r) => browser = Some(r),
                Err(e) => {
                    let next = flow::fallback_method(method, &disc, Some(&e));
                    if next == OidcSignInMethod::DeviceCode {
                        method = next;
                    } else {
                        return self.flow_failed(&id, &e);
                    }
                }
            }
        }
        if method == OidcSignInMethod::DeviceCode {
            let sink = self.clone();
            let issuer = id.clone();
            match flow::device_flow(&http, &disc, &cfg, &cancel, move |p| {
                sink.progress(
                    &issuer,
                    OidcSignInStage::DeviceCode {
                        user_code: p.user_code,
                        verification_uri: p.verification_uri,
                        expires_at: p.expires_at,
                    },
                );
            })
            .await
            {
                Ok(t) => device = Some(t),
                Err(e) => return self.flow_failed(&id, &e),
            }
        }

        self.progress(&id, OidcSignInStage::Validating);
        let (tokens, nonce) = match (browser, device) {
            (Some(b), _) => (b.tokens, Some(b.nonce)),
            (None, Some(t)) => (t, None),
            (None, None) => return self.error(&id, "token", "no tokens were obtained"),
        };
        let validator = match self.validator(&http, &disc, &cfg).await {
            Ok(v) => v,
            Err(e) => return self.error(&id, "validate", e.0),
        };
        let claims = match validator.validate(&tokens.id_token, nonce.as_deref()).await {
            Ok(c) => c,
            Err(e) => return self.error(&id, "validate", e.0),
        };
        match outcome {
            Outcome::Session => {
                self.install(&cfg, &http, &disc, Arc::new(validator), tokens, claims, "");
            }
            Outcome::Draft => self.publish_draft(&cfg, &tokens.id_token, &claims),
        }
        self.progress(&id, OidcSignInStage::Done);
        self.inner.sign_ins.lock().expect("sign_ins").remove(&id);
    }

    fn flow_failed(&self, issuer_id: &str, e: &FlowError) {
        let stage = match e {
            FlowError::Cancelled => "cancelled",
            FlowError::Timeout | FlowError::LocalUnavailable(_) => "authorize",
            FlowError::Failed(_) => "token",
        };
        self.error(issuer_id, stage, e.to_string());
        self.inner
            .sign_ins
            .lock()
            .expect("sign_ins")
            .remove(issuer_id);
    }

    /// Make `tokens` the live session and start its refresh task.
    #[allow(clippy::too_many_arguments)]
    fn install(
        &self,
        cfg: &OidcIssuerConfig,
        http: &reqwest::Client,
        disc: &Discovery,
        validator: Arc<Validator>,
        tokens: TokenResponse,
        claims: VerifiedClaims,
        previous_refresh: &str,
    ) {
        // Persist the newest refresh token before anything else can drop the old one.
        let refresh = tokens
            .refresh_token
            .clone()
            .unwrap_or_else(|| previous_refresh.to_owned());
        if !refresh.is_empty() {
            self.inner.store.set(
                SecretKind::RefreshToken,
                &cfg.id,
                &Secret::new(refresh.clone()),
            );
        }
        self.publish(cfg, &tokens.id_token, &claims, OidcIdentityState::SignedIn);

        self.stop_refresher(&cfg.id);
        if refresh.is_empty() {
            return; // no refresh token: the session lasts until `exp`
        }
        let this = self.clone();
        let (cfg, http, disc) = (cfg.clone(), http.clone(), disc.clone());
        let id = cfg.id.clone();
        let handle = tokio::spawn(async move {
            this.refresh_loop(cfg, http, disc, validator, refresh, claims.exp)
                .await;
        });
        self.inner
            .refreshers
            .lock()
            .expect("refreshers")
            .insert(id, handle);
    }

    fn publish(
        &self,
        cfg: &OidcIssuerConfig,
        id_token: &str,
        claims: &VerifiedClaims,
        state: OidcIdentityState,
    ) {
        let identity = self.store_live(cfg, id_token, claims, state);
        self.emit(SystemEvent::OidcIdentityUpdated {
            issuer_id: cfg.id.clone(),
            identity: Some(identity),
        });
    }

    /// Put the tokens in the issuer's in-memory slot and return the identity they prove.
    ///
    /// Shared by the saved path, which follows it with an identity update, and the draft path,
    /// which must not: a draft is not an issuer, so announcing an identity for it would make the
    /// UI believe an issuer it has no record of is signed in.
    fn store_live(
        &self,
        cfg: &OidcIssuerConfig,
        id_token: &str,
        claims: &VerifiedClaims,
        state: OidcIdentityState,
    ) -> OidcIdentity {
        let identity = OidcIdentity {
            issuer_id: cfg.id.clone(),
            sub: claims.sub.clone(),
            username: claims.username.clone(),
            email: claims.email.clone(),
            groups: claims.groups.clone(),
            expires_at: claims.exp,
            state,
        };
        self.slot(&cfg.id).live.store(Some(Arc::new(Live {
            id_token: Secret::new(id_token),
            identity: identity.clone(),
        })));
        identity
    }

    /// Finish a draft sign-in (045 T027, T029).
    ///
    /// Deliberately not [`Self::install`]: that writes the refresh token to the credential store
    /// before anything else, under an id that was never saved. It also starts no refresher -- the
    /// claims a draft shows are the record of one sign-in, not a session Fathom maintains.
    fn publish_draft(&self, cfg: &OidcIssuerConfig, id_token: &str, claims: &VerifiedClaims) {
        self.store_live(cfg, id_token, claims, OidcIdentityState::SignedIn);
        self.emit(SystemEvent::OidcDraftSignInSucceeded {
            result: DraftSignInResult {
                username: claims.username.clone(),
                groups: claims.groups.clone(),
                claims: claims.all_claims.clone(),
                expires_at: i64::try_from(claims.exp).unwrap_or(i64::MAX),
            },
        });
    }

    fn end_session(&self, issuer_id: &str, reason: &str) {
        self.slot(issuer_id).live.store(None);
        self.inner.store.delete(SecretKind::RefreshToken, issuer_id);
        self.emit(SystemEvent::OidcSessionEnded {
            issuer_id: issuer_id.to_owned(),
            reason: reason.to_owned(),
        });
        self.emit(SystemEvent::OidcIdentityUpdated {
            issuer_id: issuer_id.to_owned(),
            identity: None,
        });
    }

    async fn refresh_loop(
        &self,
        cfg: OidcIssuerConfig,
        http: reqwest::Client,
        disc: Discovery,
        validator: Arc<Validator>,
        mut refresh: String,
        mut exp: u64,
    ) {
        loop {
            let due = exp.saturating_sub(self.inner.refresh_lead.as_secs());
            let wait =
                Duration::from_secs(due.saturating_sub(now_secs())).max(Duration::from_millis(200));
            tokio::time::sleep(wait).await;

            // Mark Refreshing without dropping the still-valid token.
            if let Some(live) = self.slot(&cfg.id).live.load_full() {
                let mut identity = live.identity.clone();
                identity.state = OidcIdentityState::Refreshing;
                self.emit(SystemEvent::OidcIdentityUpdated {
                    issuer_id: cfg.id.clone(),
                    identity: Some(identity),
                });
            }

            match flow::refresh(&http, &disc, &cfg, &refresh).await {
                Ok(tokens) => match validator.validate(&tokens.id_token, None).await {
                    Ok(claims) => {
                        if let Some(rt) = &tokens.refresh_token {
                            self.inner.store.set(
                                SecretKind::RefreshToken,
                                &cfg.id,
                                &Secret::new(rt.clone()),
                            );
                            refresh.clone_from(rt);
                        }
                        exp = claims.exp;
                        self.publish(&cfg, &tokens.id_token, &claims, OidcIdentityState::SignedIn);
                    }
                    Err(e) => return self.end_session(&cfg.id, &e.0),
                },
                Err(e) if e.code == "transport" && exp > now_secs() => {
                    // Offline but the token is still good: try again shortly.
                    tokio::time::sleep(TRANSIENT_RETRY).await;
                    exp = exp.min(
                        now_secs() + TRANSIENT_RETRY.as_secs() + self.inner.refresh_lead.as_secs(),
                    );
                }
                Err(e) => return self.end_session(&cfg.id, &e.message),
            }
        }
    }
}

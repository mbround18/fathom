//! Runtime-level boundary tests for 038 (Principle III): the full command -> event flow
//! against stub provider servers, an in-memory keyring double and a recording transport.
//! No real provider, keyring or network is touched.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Receiver;
use secrecy::{ExposeSecret, SecretString};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::http::AiHttp;
use super::store::testing::{Fixture, Mode};
use super::{AiRuntime, Emitter, Endpoints};
use crate::protocol::v1::{
    ApiKeyInput, BlockedReason, CredentialMigrationOutcome, LoginPhase, Provider, ProviderState,
    ProviderStatus,
};
use crate::protocol::{BackendCmd, BackendEvent, BackendEventEnvelope, SystemEvent};

struct Rig {
    rt: AiRuntime,
    fx: Fixture,
    dir: tempfile::TempDir,
    em: Emitter,
    rx: Receiver<BackendEventEnvelope>,
    log: Mutex<Vec<SystemEvent>>,
    http: Arc<AiHttp>,
}

fn endpoints(uri: &str) -> Endpoints {
    Endpoints {
        claude_base: uri.to_owned(),
        openai_base: uri.to_owned(),
        github_oauth_base: uri.to_owned(),
        github_api_base: uri.to_owned(),
        github_chat_url: format!("{uri}/gh/chat/completions"),
        openai_chat_url: format!("{uri}/oa/chat/completions"),
        github_client_id: Some("cid".to_owned()),
        time_unit: Duration::from_millis(20),
    }
}

fn rig(ep: Endpoints) -> Rig {
    let fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(AiHttp::new().with_recorder());
    let rt = AiRuntime::new(
        Arc::clone(&fx.store),
        Arc::clone(&http),
        ep,
        Some(dir.path().join("config.json")),
        None,
    );
    let (tx, rx) = crossbeam_channel::unbounded();
    let em = Emitter::new(Arc::new(std::sync::atomic::AtomicU64::new(0)), tx);
    Rig {
        rt,
        fx,
        dir,
        em,
        rx,
        log: Mutex::new(Vec::new()),
        http,
    }
}

impl Rig {
    fn pump(&self) {
        let mut log = self.log.lock().unwrap();
        while let Ok(env) = self.rx.try_recv() {
            if let BackendEvent::System(ev) = env.event {
                log.push(ev);
            }
        }
    }

    fn events(&self) -> Vec<SystemEvent> {
        self.pump();
        self.log.lock().unwrap().clone()
    }

    async fn wait_until(&self, what: &str, pred: impl Fn(&[SystemEvent]) -> bool) {
        for _ in 0..250 {
            if pred(&self.events()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting for {what}; events: {:#?}", self.events());
    }

    fn status(&self, provider: Provider) -> ProviderStatus {
        match self.rt.status_event() {
            SystemEvent::AiProviderStatus { statuses, .. } => statuses
                .into_iter()
                .find(|s| s.provider == provider)
                .unwrap(),
            _ => unreachable!(),
        }
    }

    fn error_codes(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                SystemEvent::AiError { code, .. } => Some(code),
                _ => None,
            })
            .collect()
    }

    fn all_json(&self) -> String {
        self.events()
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn has_error(events: &[SystemEvent], wanted: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, SystemEvent::AiError { code, .. } if code == wanted))
}

fn flow_phase(events: &[SystemEvent], f: impl Fn(&LoginPhase) -> bool) -> bool {
    events
        .iter()
        .any(|e| matches!(e, SystemEvent::AiLoginFlowUpdate { flow } if f(&flow.phase)))
}

async fn mount_github(server: &MockServer, token_script: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/login/device/code"))
        .and(body_string_contains("client_id=cid"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code":"dev","user_code":"ABCD-1234",
            "verification_uri":"https://github.com/login/device","expires_in":600,"interval":1
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_script))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer gho_secret_token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"login":"octocat"})),
        )
        .mount(server)
        .await;
}

// ------------------------------------------------------------------ US1 login

#[tokio::test]
async fn github_device_login_end_to_end() {
    let server = MockServer::start().await;
    mount_github(
        &server,
        serde_json::json!({"access_token":"gho_secret_token"}),
    )
    .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.login_start(Provider::Github, &r.em).await;
    let first = r.events();
    assert!(
        first
            .iter()
            .any(|e| matches!(e, SystemEvent::AiLoginFlowUpdate { flow }
            if flow.user_code.as_deref() == Some("ABCD-1234")
            && flow.verification_uri.as_deref() == Some("https://github.com/login/device")
            && flow.phase == LoginPhase::Pending)),
        "{first:#?}"
    );
    r.wait_until("connected", |_| {
        r.status(Provider::Github).state == ProviderState::Connected
    })
    .await;
    assert_eq!(
        r.status(Provider::Github).account_label.as_deref(),
        Some("octocat")
    );
    assert_eq!(
        r.fx.keyring.raw("ai-assist/github").as_deref(),
        Some("gho_secret_token")
    );
    assert!(flow_phase(&r.events(), |p| *p == LoginPhase::Succeeded));
    assert_eq!(r.rt.settings().active_provider, Some(Provider::Github));
    // The token never crossed the event boundary or the config file.
    assert!(!r.all_json().contains("gho_secret_token"));
    let cfg = std::fs::read_to_string(r.dir.path().join("config.json")).unwrap();
    assert!(!cfg.contains("gho_secret_token"));
}

#[tokio::test]
async fn github_denied_ends_flow_and_allows_retry() {
    let server = MockServer::start().await;
    mount_github(&server, serde_json::json!({"error":"access_denied"})).await;
    let r = rig(endpoints(&server.uri()));
    r.rt.login_start(Provider::Github, &r.em).await;
    r.wait_until("denied", |ev| flow_phase(ev, |p| *p == LoginPhase::Denied))
        .await;
    assert_ne!(r.status(Provider::Github).state, ProviderState::Connected);
    assert!(r.fx.keyring.raw("ai-assist/github").is_none());
    // Flow slot is free again.
    assert!(r.rt.session().flow.is_none());
}

#[tokio::test]
async fn second_login_while_one_is_in_flight_is_refused_and_cancel_works() {
    let server = MockServer::start().await;
    mount_github(
        &server,
        serde_json::json!({"error":"authorization_pending"}),
    )
    .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.login_start(Provider::Github, &r.em).await;
    r.rt.login_start(Provider::Github, &r.em).await;
    assert!(has_error(&r.events(), "login_in_progress"));
    r.rt.login_cancel(&r.em);
    assert!(r.rt.session().flow.is_none());
    assert!(flow_phase(&r.events(), |p| matches!(
        p,
        LoginPhase::Failed { .. }
    )));
    assert_ne!(r.status(Provider::Github).state, ProviderState::Connecting);
    // A new login can start after cancel.
    r.rt.login_start(Provider::Github, &r.em).await;
    assert!(r.rt.session().flow.is_some());
    r.rt.login_cancel(&r.em);
}

#[tokio::test]
async fn github_without_client_id_gives_actionable_error() {
    let server = MockServer::start().await;
    let mut ep = endpoints(&server.uri());
    ep.github_client_id = None;
    let r = rig(ep);
    r.rt.login_start(Provider::Github, &r.em).await;
    let events = r.events();
    assert!(
        has_error(&events, "github_client_id_missing"),
        "{events:#?}"
    );
    assert!(r.rt.session().flow.is_none());
}

#[tokio::test]
async fn claude_key_valid_is_stored_and_status_connected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-api-key", "sk-ant-SECRET-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})))
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.submit_api_key(
        Provider::Claude,
        ApiKeyInput("  sk-ant-SECRET-123 ".into()),
        &r.em,
    )
    .await;
    assert_eq!(r.status(Provider::Claude).state, ProviderState::Connected);
    assert_eq!(
        r.fx.keyring.raw("ai-assist/claude").as_deref(),
        Some("sk-ant-SECRET-123")
    );
    assert!(flow_phase(&r.events(), |p| *p == LoginPhase::Succeeded));
}

#[tokio::test]
async fn invalid_key_is_rejected_verbatim_and_never_stored() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(r#"{"error":{"message":"Incorrect API key provided"}}"#),
        )
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.submit_api_key(Provider::Openai, ApiKeyInput("sk-bad".into()), &r.em)
        .await;
    let events = r.events();
    let verbatim = events.iter().find_map(|e| match e {
        SystemEvent::AiError {
            code,
            message_verbatim,
            ..
        } if code == "credential_rejected" => Some(message_verbatim.clone()),
        _ => None,
    });
    assert!(verbatim.unwrap().contains("Incorrect API key provided"));
    assert!(r.fx.keyring.raw("ai-assist/openai").is_none());
    assert_eq!(
        r.status(Provider::Openai).state,
        ProviderState::NotConfigured
    );
}

#[tokio::test]
async fn rate_limited_key_validation_reports_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "17")
                .set_body_string("slow down"),
        )
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("k".into()), &r.em)
        .await;
    let hint = r.events().into_iter().find_map(|e| match e {
        SystemEvent::AiError { code, hint, .. } if code == "rate_limited" => Some(hint),
        _ => None,
    });
    assert!(hint.unwrap().contains("17"));
    assert!(r.fx.keyring.raw("ai-assist/claude").is_none());
}

#[tokio::test]
async fn empty_key_is_refused_without_network() {
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("   ".into()), &r.em)
        .await;
    assert!(has_error(&r.events(), "empty_key"));
    assert!(r.http.recorded().is_empty());
}

#[tokio::test]
async fn sign_out_deletes_credential_and_github_says_revoke_is_manual() {
    let server = MockServer::start().await;
    mount_github(
        &server,
        serde_json::json!({"access_token":"gho_secret_token"}),
    )
    .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.login_start(Provider::Github, &r.em).await;
    r.wait_until("connected", |_| {
        r.status(Provider::Github).state == ProviderState::Connected
    })
    .await;
    r.rt.sign_out(Provider::Github, &r.em).await;
    assert!(r.fx.keyring.raw("ai-assist/github").is_none());
    assert_eq!(
        r.status(Provider::Github).state,
        ProviderState::NotConfigured
    );
    assert_eq!(r.rt.settings().active_provider, None);
    assert!(has_error(&r.events(), "github_revoke_manual"));
}

#[tokio::test]
async fn needs_reauth_is_set_by_rejection_and_cleared_by_new_login() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("k1".into()), &r.em)
        .await;
    r.rt.mark_needs_reauth(Provider::Claude, &r.em);
    assert_eq!(r.status(Provider::Claude).state, ProviderState::NeedsReauth);
    // The assistant refuses to use a rejected credential.
    let err = r.rt.turn_client().await.err().expect("needs reauth");
    assert!(err.to_string().contains("sign in again"));
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("k2".into()), &r.em)
        .await;
    assert_eq!(r.status(Provider::Claude).state, ProviderState::Connected);
}

// ------------------------------------------------------------------ US2 storage

#[tokio::test]
async fn locked_store_blocks_login_and_nothing_is_written() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    r.fx.keyring.set_mode(Mode::Locked);
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("k".into()), &r.em)
        .await;
    assert!(has_error(&r.events(), "store_locked"));
    assert_eq!(
        r.status(Provider::Claude).state,
        ProviderState::Blocked {
            reason: BlockedReason::StoreLocked
        }
    );
    assert!(
        !r.fx.file_path().exists(),
        "must not fall back to a file when locked"
    );
}

#[tokio::test]
async fn unavailable_store_refuses_until_file_optin() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    r.fx.keyring.set_mode(Mode::Unavailable);
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("k".into()), &r.em)
        .await;
    assert!(has_error(&r.events(), "store_unavailable"));
    assert!(!r.fx.file_path().exists());
    // Opt in explicitly (without confirmation nothing changes).
    r.rt.enable_file_store(false, &r.em).await;
    assert!(!r.rt.settings().file_store_optin);
    r.rt.enable_file_store(true, &r.em).await;
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput("k".into()), &r.em)
        .await;
    assert_eq!(r.status(Provider::Claude).state, ProviderState::Connected);
    assert!(r.fx.file_path().exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(r.fx.file_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[tokio::test]
async fn startup_reports_a_migration_once() {
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.session().migration_notice = Some(CredentialMigrationOutcome::Migrated);
    r.rt.get_status(&r.em).await;
    r.rt.get_status(&r.em).await;
    let n = r
        .events()
        .iter()
        .filter(|e| matches!(e, SystemEvent::AiCredentialMigration { .. }))
        .count();
    assert_eq!(n, 1);
}

/// T011: neither events, the config file, `Debug` of the command, nor captured logs ever
/// contain the key.
#[tokio::test]
async fn secrets_never_leak_into_events_config_debug_or_logs() {
    use std::io::Write;
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }
    let buf = Buf::default();
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish(),
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    let r = rig(endpoints(&server.uri()));
    let secret = "sk-ant-SECRET-LEAKCHECK-999";
    let cmd = BackendCmd::AiProviderSubmitApiKey {
        provider: Provider::Claude,
        key: ApiKeyInput(secret.into()),
    };
    assert!(!format!("{cmd:?}").contains(secret));
    r.rt.submit_api_key(Provider::Claude, ApiKeyInput(secret.into()), &r.em)
        .await;
    r.rt.get_status(&r.em).await;
    assert!(!r.all_json().contains(secret));
    let cfg = std::fs::read_to_string(r.dir.path().join("config.json")).unwrap();
    assert!(!cfg.contains(secret));
    let logs = String::from_utf8_lossy(&buf.0.lock().unwrap()).to_string();
    assert!(!logs.contains(secret), "secret leaked into logs");
    // Same for the failure path.
    r.fx.keyring.set_mode(Mode::Locked);
    r.rt.submit_api_key(Provider::Openai, ApiKeyInput(secret.into()), &r.em)
        .await;
    assert!(!r.all_json().contains(secret));
    let logs = String::from_utf8_lossy(&buf.0.lock().unwrap()).to_string();
    assert!(!logs.contains(secret));
    let _ = SecretString::from(secret.to_owned()).expose_secret();
}

// ------------------------------------------------------------------ US3 ollama

async fn mount_ollama(server: &MockServer, models: &[&str]) {
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"version":"0.5.1"})),
        )
        .mount(server)
        .await;
    let list: Vec<_> = models
        .iter()
        .map(
            |m| serde_json::json!({"name": m, "size": 1u64, "modified_at": "2025-01-01T00:00:00Z"}),
        )
        .collect();
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"models": list})))
        .mount(server)
        .await;
}

fn last_ollama(events: &[SystemEvent]) -> (crate::protocol::v1::OllamaState, Option<String>) {
    events
        .iter()
        .rev()
        .find_map(|e| match e {
            SystemEvent::AiOllamaState {
                state,
                selected_model,
            } => Some((state.clone(), selected_model.clone())),
            _ => None,
        })
        .expect("an AiOllamaState event")
}

#[tokio::test]
async fn ollama_detected_pick_persists_and_drives_a_turn() {
    let server = MockServer::start().await;
    mount_ollama(&server, &["llama3.2:latest", "qwen2.5:7b"]).await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_string_contains("\"model\":\"qwen2.5:7b\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"message":{"role":"assistant","content":"local hello"}}),
        ))
        .mount(&server)
        .await;
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.session().settings.ollama_endpoint = server.uri();
    r.rt.ollama_probe(None, &r.em).await;
    let (state, sel) = last_ollama(&r.events());
    assert!(state.reachable);
    assert_eq!(state.models.len(), 2);
    assert_eq!(sel, None);

    r.rt.ollama_select_model("qwen2.5:7b".into(), &r.em);
    assert_eq!(r.status(Provider::Ollama).state, ProviderState::Connected);
    assert_eq!(r.rt.settings().active_provider, Some(Provider::Ollama));
    // Survives a restart: settings are on disk.
    let on_disk =
        crate::backend::persistence::load_ai_settings_at(&r.dir.path().join("config.json"));
    assert_eq!(on_disk.ollama_model.as_deref(), Some("qwen2.5:7b"));

    let client = r.rt.turn_client().await.expect("ollama client");
    let reply = client.send_turn("sys", &[]).await.unwrap();
    assert_eq!(reply.text, "local hello");
}

#[tokio::test]
async fn ollama_not_running_reports_state_not_a_crash() {
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.session().settings.ollama_endpoint = "http://127.0.0.1:1".into();
    r.rt.ollama_probe(None, &r.em).await;
    let (state, _) = last_ollama(&r.events());
    assert!(!state.reachable);
    assert!(state.error.is_some());
}

#[tokio::test]
async fn ollama_with_no_models_is_reachable_and_empty() {
    let server = MockServer::start().await;
    mount_ollama(&server, &[]).await;
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.session().settings.ollama_endpoint = server.uri();
    r.rt.ollama_probe(None, &r.em).await;
    let (state, _) = last_ollama(&r.events());
    assert!(state.reachable && state.models.is_empty() && state.error.is_none());
}

#[tokio::test]
async fn removed_selected_model_is_cleared_with_guidance_on_reprobe() {
    let server = MockServer::start().await;
    mount_ollama(&server, &["llama3.2:latest"]).await;
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.session().settings.ollama_endpoint = server.uri();
    r.rt.ollama_select_model("gone:1b".into(), &r.em);
    r.rt.ollama_probe(None, &r.em).await;
    assert_eq!(r.rt.settings().ollama_model, None);
    assert!(has_error(&r.events(), "ollama_model_missing"));
    let (_, sel) = last_ollama(&r.events());
    assert_eq!(sel, None);
}

#[tokio::test]
async fn model_404_during_a_turn_clears_selection_and_keeps_verbatim_text() {
    let server = MockServer::start().await;
    mount_ollama(&server, &["other"]).await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(404).set_body_string(r#"{"error":"model \"ghost\" not found"}"#),
        )
        .mount(&server)
        .await;
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.session().settings.ollama_endpoint = server.uri();
    r.rt.ollama_select_model("ghost".into(), &r.em);
    let err =
        r.rt.turn_client()
            .await
            .ok()
            .unwrap()
            .send_turn("s", &[])
            .await
            .unwrap_err();
    assert!(err.to_string().contains("model \\\"ghost\\\" not found"));
    match err.downcast_ref::<crate::backend::agent::TurnFailure>() {
        Some(crate::backend::agent::TurnFailure::OllamaModelMissing { model, .. }) => {
            r.rt.model_missing(model, &r.em).await;
        }
        other => panic!("expected typed failure, got {other:?}"),
    }
    assert_eq!(r.rt.settings().ollama_model, None);
    assert!(has_error(&r.events(), "ollama_model_missing"));
}

#[tokio::test]
async fn lan_ollama_endpoint_needs_explicit_confirmation() {
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.set_ollama_endpoint("http://192.168.1.50:11434".into(), false, &r.em)
        .await;
    assert!(has_error(&r.events(), "lan_confirmation_required"));
    assert_eq!(r.rt.settings().ollama_endpoint, "http://127.0.0.1:11434");
    r.rt.set_ollama_endpoint("not a url".into(), false, &r.em)
        .await;
    assert!(has_error(&r.events(), "invalid_endpoint"));
}

// ------------------------------------------------------------------ US4 offline

#[tokio::test]
async fn offline_mode_sends_zero_non_loopback_requests_across_a_full_turn() {
    let server = MockServer::start().await;
    mount_ollama(&server, &["llama3.2"]).await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"message":{"role":"assistant","content":"offline ok"}}),
        ))
        .mount(&server)
        .await;
    // Hosted endpoints point at a non-loopback address that would hang if ever contacted.
    let r = rig(endpoints("http://192.0.2.1:9"));
    r.fx.store
        .save(Provider::Claude, &SecretString::from("sk-ant".to_owned()))
        .unwrap();
    r.rt.refresh_presence().await;
    r.rt.set_active_provider(Some(Provider::Claude), &r.em)
        .await;
    r.rt.session().settings.ollama_endpoint = server.uri();
    r.rt.ollama_select_model("llama3.2".into(), &r.em);

    r.rt.set_offline_mode(true, &r.em);
    // Hosted login and key submission are refused with offline_blocked.
    r.rt.login_start(Provider::Github, &r.em).await;
    r.rt.submit_api_key(Provider::Openai, ApiKeyInput("k".into()), &r.em)
        .await;
    assert_eq!(
        r.error_codes()
            .iter()
            .filter(|c| *c == "offline_blocked")
            .count(),
        2
    );
    // Effective provider is Ollama despite Claude being active; hosted are Blocked.
    assert_eq!(r.rt.effective_provider(), Some(Provider::Ollama));
    assert_eq!(
        r.status(Provider::Claude).state,
        ProviderState::Blocked {
            reason: BlockedReason::Offline
        }
    );
    // A full assistant turn works, locally.
    r.rt.ollama_probe(None, &r.em).await;
    let reply =
        r.rt.turn_client()
            .await
            .ok()
            .unwrap()
            .send_turn("s", &[])
            .await
            .unwrap();
    assert_eq!(reply.text, "offline ok");
    // Even a hosted call forced through the chokepoint is refused before any I/O.
    let refused = r
        .http
        .get("http://192.0.2.1:9/v1/models")
        .unwrap()
        .send()
        .await;
    assert!(refused.err().unwrap().is_offline_blocked());

    let recorded = r.http.recorded();
    assert!(!recorded.is_empty());
    for req in &recorded {
        if req.host == "192.0.2.1" {
            assert!(!req.allowed, "non-loopback request was allowed: {req:?}");
        } else {
            assert!(req.allowed && req.loopback, "{req:?}");
        }
    }

    // Toggling off restores the hosted provider.
    r.rt.set_offline_mode(false, &r.em);
    assert_eq!(r.rt.effective_provider(), Some(Provider::Claude));
    assert_eq!(r.status(Provider::Claude).state, ProviderState::Connected);
}

#[tokio::test]
async fn offline_without_a_model_never_falls_back_to_hosted() {
    let r = rig(endpoints("http://192.0.2.1:9"));
    r.rt.session().settings.active_provider = Some(Provider::Claude);
    r.rt.set_offline_mode(true, &r.em);
    let err = r.rt.turn_client().await.err().expect("must refuse");
    assert!(err.to_string().contains("Offline mode is on"));
    assert!(r.http.recorded().is_empty());
}

#[tokio::test]
async fn offline_setting_survives_restart() {
    let r = rig(endpoints("http://127.0.0.1:1"));
    r.rt.set_offline_mode(true, &r.em);
    let rt2 = AiRuntime::new(
        Arc::clone(&r.fx.store),
        Arc::new(AiHttp::new()),
        endpoints("http://127.0.0.1:1"),
        Some(r.dir.path().join("config.json")),
        None,
    );
    rt2.bootstrap(&r.em).await;
    assert!(rt2.settings().offline_mode);
    assert!(rt2.http().is_offline());
}

// ------------------------------------------------------------------ grep gate

/// T023: no agent/AI code builds its own HTTP client; everything goes through `AiHttp`.
#[test]
fn only_http_rs_may_construct_a_reqwest_client() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend");
    let mut files = Vec::new();
    walk(&root.join("agent"), &mut files);
    walk(&root.join("ai"), &mut files);
    let needles = [
        ["reqwest", "::Client"].concat(),
        ["reqwest", "::get("].concat(),
        ["reqwest", "::blocking"].concat(),
    ];
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        if name == "http.rs" || name == "tests.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for needle in &needles {
            assert!(
                !text.contains(needle.as_str()),
                "{} constructs its own HTTP client ({needle}); use AiHttp",
                file.display()
            );
        }
    }
}

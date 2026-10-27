//! Passkey commands (spec 055 T028, the 054 shapes): registration, verification, step-up on
//! `Confirm`, removal and recovery, with a software passkey. Every refusal and every passkey
//! change is audited; removal and recovery end the person's other sessions.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::TestDb;
use http_body_util::BodyExt;
use keel_core::ports::{Clock, TestClock};
use keel_protocol::{
    AssertionCredential, AttestationCredential, CeremonyId, Cmd, CmdEnvelope, Event, EventEnvelope,
    PasskeyAssertion, PasskeyCredential, PasskeyName, PasskeyPurpose, PasskeyRefusedReason,
    PasskeyResponse, RecoveryCode, RequestId, Verification,
};
use keel_server::config::{Config, Flags};
use keel_server::passkeys::PgStore;
use keel_server::serve::{self, AppState};
use keel_server::signin::{self, Established};
use secrecy::SecretString;
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;
use web_auth::session::{Identity, Provider, Tokens};
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

const ORIGIN: &str = "https://keel.example.com";

type Authenticator = WebauthnAuthenticator<SoftPasskey>;

struct Harness {
    db: TestDb,
    app: AppState,
    clock: Arc<TestClock>,
}

impl Harness {
    async fn new(test: &str) -> Option<Self> {
        let db = TestDb::new(test).await?;
        let env: HashMap<&str, String> = HashMap::from([
            ("KEEL_PUBLIC_URL", ORIGIN.to_owned()),
            ("KEEL_DATABASE_URL", "postgres://unused/keel".to_owned()),
            ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key".to_owned()),
            ("KEEL_GITHUB_CLIENT_ID", "gh-id".to_owned()),
            ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret".to_owned()),
        ]);
        let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
        let providers = serve::build_providers(&cfg).await.expect("providers");
        let clock = Arc::new(TestClock::new(OffsetDateTime::now_utc()));
        let app = AppState::new(Arc::new(cfg), providers, db.app.clone(), clock.clone());
        Some(Self { db, app, clock })
    }

    async fn sign_in(&self, sub: &str, name: &str) -> Established {
        let who = Identity {
            provider: Provider::Github,
            issuer: "https://github.com".into(),
            sub: sub.into(),
            username: name.into(),
            groups: vec![],
            email: None,
        };
        let tokens = Tokens {
            id_token: SecretString::from(String::new()),
            access_token: SecretString::from("provider-access-token".to_owned()),
            refresh_token: None,
            expires_at: tokio::time::Instant::now() + StdDuration::from_secs(3600),
        };
        signin::establish(&self.app, who, tokens, json!({ "provider": "github" }))
            .await
            .unwrap_or_else(|e| panic!("establish: {e:?}"))
    }

    /// Runs `cmd` over `POST /api/v1/cmd`; the status and the reply events.
    async fn try_cmd(&self, est: &Established, cmd: Cmd) -> (StatusCode, Vec<Event>) {
        let pair = est.cookies[0].to_str().expect("ascii");
        let pair = pair.split(';').next().expect("pair").to_owned();
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(1),
            cmd,
        })
        .expect("json");
        let req = Request::post("/api/v1/cmd")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, pair)
            .header("x-csrf-token", &est.csrf)
            .body(Body::from(body))
            .expect("req");
        let r = serve::router(self.app.clone())
            .oneshot(req)
            .await
            .expect("infallible");
        let status = r.status();
        let body = r.into_body().collect().await.expect("body").to_bytes();
        let events = serde_json::from_slice::<Vec<EventEnvelope>>(&body)
            .map(|v| v.into_iter().map(|e| e.event).collect())
            .unwrap_or_default();
        (status, events)
    }

    async fn cmd(&self, est: &Established, cmd: Cmd) -> Vec<Event> {
        let (status, events) = self.try_cmd(est, cmd).await;
        assert_eq!(status, StatusCode::OK);
        events
    }

    /// Ages the session's last passkey check past the step-up window.
    async fn go_stale(&self, est: &Established) {
        sqlx::query("update sessions set verified_at = null where id = $1")
            .bind(&est.session)
            .execute(&self.db.owner)
            .await
            .expect("stale");
    }

    async fn audit(&self) -> Vec<Value> {
        let rows: Vec<(Value,)> = sqlx::query_as(
            "select to_jsonb(a) - 'hash' - 'prev_hash' from audit_event a order by seq",
        )
        .fetch_all(&self.db.owner)
        .await
        .expect("audit rows");
        rows.into_iter().map(|r| r.0).collect()
    }

    async fn audited(&self, action: &str) -> Vec<Value> {
        self.audit()
            .await
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }

    /// Begins a ceremony and returns its id and options.
    async fn begin(&self, est: &Established, purpose: PasskeyPurpose) -> (CeremonyId, Value) {
        let events = self.cmd(est, Cmd::PasskeyBegin { purpose }).await;
        match events.as_slice() {
            [Event::PasskeyChallenge(r)] => (r.ceremony, r.options.clone()),
            other => panic!("expected a challenge: {other:?}"),
        }
    }

    /// Registers a passkey named `name` with `auth`; the reply events.
    async fn register(
        &self,
        est: &Established,
        auth: &mut Authenticator,
        name: &str,
    ) -> Vec<Event> {
        let (ceremony, options) = self.begin(est, register(name)).await;
        let credential = attest(auth, ORIGIN, &options);
        self.cmd(
            est,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony,
                credential: PasskeyCredential::Attestation(credential),
            }),
        )
        .await
    }

    /// Starts a `verify` ceremony and answers it with `auth`, without sending the answer.
    async fn assertion(&self, est: &Established, auth: &mut Authenticator) -> PasskeyAssertion {
        let (ceremony, options) = self.begin(est, PasskeyPurpose::Verify).await;
        PasskeyAssertion {
            ceremony,
            credential: assert_with(auth, ORIGIN, &options),
        }
    }

    async fn verification(&self, est: &Established) -> Verification {
        match self.cmd(est, Cmd::GetVerification).await.as_slice() {
            [Event::Verification(v)] => v.clone(),
            other => panic!("expected Verification: {other:?}"),
        }
    }
}

fn authenticator() -> Authenticator {
    // `true` lets the soft token claim user verification, which passkeys require.
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

fn register(name: &str) -> PasskeyPurpose {
    PasskeyPurpose::Register {
        name: PasskeyName::new(name).expect("name"),
    }
}

/// WebAuthn's JSON as a browser would send it: no nulls, no extension results.
fn browser_json(mut v: Value) -> Value {
    let obj = v.as_object_mut().expect("credential object");
    obj.remove("extensions");
    let response = obj["response"].as_object_mut().expect("response");
    response.retain(|_, f| !f.is_null());
    v
}

fn attest(auth: &mut Authenticator, origin: &str, options: &Value) -> AttestationCredential {
    let ccr: CreationChallengeResponse =
        serde_json::from_value(json!({ "publicKey": options })).expect("creation options");
    let reg = auth
        .do_registration(Url::parse(origin).unwrap(), ccr)
        .expect("softpasskey registers");
    serde_json::from_value(browser_json(serde_json::to_value(reg).unwrap()))
        .expect("protocol attestation")
}

fn assert_with(auth: &mut Authenticator, origin: &str, options: &Value) -> AssertionCredential {
    let rcr: RequestChallengeResponse =
        serde_json::from_value(json!({ "publicKey": options })).expect("request options");
    let cred = auth
        .do_authentication(Url::parse(origin).unwrap(), rcr)
        .expect("softpasskey asserts");
    serde_json::from_value(browser_json(serde_json::to_value(cred).unwrap()))
        .expect("protocol assertion")
}

fn refused(events: &[Event]) -> PasskeyRefusedReason {
    match events {
        [Event::PasskeyRefused { reason }] => *reason,
        other => panic!("expected PasskeyRefused: {other:?}"),
    }
}

#[tokio::test]
async fn first_passkey_registers_without_a_check_and_verifies_the_session() {
    let Some(h) = Harness::new("pk_first").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let v = h.verification(&alice).await;
    assert!(!v.verified && !v.enrolled && !v.recovering);
    assert!(v.required, "every role verifies before step-up actions");

    let mut auth = authenticator();
    let events = h.register(&alice, &mut auth, "laptop").await;
    let [Event::PasskeyRegistered(info), Event::Verification(v)] = events.as_slice() else {
        panic!("expected PasskeyRegistered then Verification: {events:?}");
    };
    assert_eq!(info.name, "laptop");
    assert_eq!(info.last_used, None);
    assert!(v.verified && v.enrolled && v.at.is_some());

    match h.cmd(&alice, Cmd::ListPasskeys).await.as_slice() {
        [Event::Passkeys { items }] => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].id, info.id);
        }
        other => panic!("expected Passkeys: {other:?}"),
    }
    // GetMe tells the same story.
    let [Event::Me(me)] = <[Event; 1]>::try_from(h.cmd(&alice, Cmd::GetMe).await).unwrap() else {
        panic!("expected Me");
    };
    assert!(me.verification.verified && me.verification.enrolled);

    let rows = h.audited("passkey_registered").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["decision"], "allowed");
    assert_eq!(rows[0]["target"]["name"], "laptop");
    // No credential material, challenge or session in the audit log.
    let all = serde_json::to_string(&h.audit().await).unwrap();
    for secret in [
        "attestationObject",
        "clientDataJSON",
        "challenge",
        &alice.session,
    ] {
        assert!(!all.contains(secret), "audit log holds {secret}");
    }
    h.db.drop().await;
}

#[tokio::test]
async fn later_passkeys_need_a_fresh_check() {
    let Some(h) = Harness::new("pk_later").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let mut auth = authenticator();
    h.register(&alice, &mut auth, "laptop").await;
    h.go_stale(&alice).await;
    assert!(!h.verification(&alice).await.verified);

    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyBegin {
                purpose: register("phone"),
            },
        )
        .await;
    assert!(
        matches!(events.as_slice(), [Event::OpError { code, .. }] if code == "step_up_required"),
        "{events:?}"
    );

    // Verify, then register the second one.
    let a = h.assertion(&alice, &mut auth).await;
    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony: a.ceremony,
                credential: PasskeyCredential::Assertion(a.credential),
            }),
        )
        .await;
    let [Event::Verification(v)] = events.as_slice() else {
        panic!("expected Verification: {events:?}");
    };
    assert!(v.verified);
    assert_eq!(h.audited("passkey_verified").await.len(), 1);
    let mut phone = authenticator();
    let events = h.register(&alice, &mut phone, "phone").await;
    assert!(
        matches!(events.first(), Some(Event::PasskeyRegistered(_))),
        "{events:?}"
    );

    // A name already used is refused.
    let (ceremony, options) = h.begin(&alice, register("phone")).await;
    let mut third = authenticator();
    let credential = attest(&mut third, ORIGIN, &options);
    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony,
                credential: PasskeyCredential::Attestation(credential),
            }),
        )
        .await;
    assert_eq!(refused(&events), PasskeyRefusedReason::Invalid);
    h.db.drop().await;
}

#[tokio::test]
async fn verify_without_a_passkey_asks_for_enrolment() {
    let Some(h) = Harness::new("pk_enrol").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyBegin {
                purpose: PasskeyPurpose::Verify,
            },
        )
        .await;
    assert_eq!(events, vec![Event::PasskeyEnrolmentRequired]);
    h.db.drop().await;
}

#[tokio::test]
async fn ceremonies_are_single_use_bound_to_the_session_and_expire() {
    let Some(h) = Harness::new("pk_ceremony").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let mut auth = authenticator();
    h.register(&alice, &mut auth, "laptop").await;

    // Used twice.
    let a = h.assertion(&alice, &mut auth).await;
    let finish = Cmd::PasskeyFinish(PasskeyResponse {
        ceremony: a.ceremony,
        credential: PasskeyCredential::Assertion(a.credential),
    });
    assert!(matches!(
        h.cmd(&alice, finish.clone()).await.as_slice(),
        [Event::Verification(_)]
    ));
    assert_eq!(
        refused(&h.cmd(&alice, finish).await),
        PasskeyRefusedReason::Expired
    );

    // Another session's ceremony is unknown here.
    let alice2 = h.sign_in("1", "alice").await;
    let a = h.assertion(&alice, &mut auth).await;
    let events = h
        .cmd(
            &alice2,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony: a.ceremony,
                credential: PasskeyCredential::Assertion(a.credential),
            }),
        )
        .await;
    assert_eq!(refused(&events), PasskeyRefusedReason::Expired);

    // A registration answer to a verify ceremony is refused.
    let (ceremony, _) = h.begin(&alice, PasskeyPurpose::Verify).await;
    let (_, reg_options) = h.begin(&alice, register("other")).await;
    let mut other = authenticator();
    let wrong = attest(&mut other, ORIGIN, &reg_options);
    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony,
                credential: PasskeyCredential::Attestation(wrong),
            }),
        )
        .await;
    assert_eq!(refused(&events), PasskeyRefusedReason::Expired);

    // Older than two minutes.
    let a = h.assertion(&alice, &mut auth).await;
    h.clock
        .advance(time::Duration::minutes(2) + time::Duration::seconds(1));
    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony: a.ceremony,
                credential: PasskeyCredential::Assertion(a.credential),
            }),
        )
        .await;
    assert_eq!(refused(&events), PasskeyRefusedReason::Expired);

    // Made for another site (a subdomain the authenticator allows, Keel does not).
    let (ceremony, options) = h.begin(&alice, PasskeyPurpose::Verify).await;
    let credential = assert_with(&mut auth, "https://evil.keel.example.com", &options);
    let events = h
        .cmd(
            &alice,
            Cmd::PasskeyFinish(PasskeyResponse {
                ceremony,
                credential: PasskeyCredential::Assertion(credential),
            }),
        )
        .await;
    assert_eq!(refused(&events), PasskeyRefusedReason::WrongOrigin);

    // Every refusal is audited with its reason.
    let reasons: Vec<Value> = h
        .audited("passkey_finish")
        .await
        .into_iter()
        .filter(|r| r["decision"] == "denied")
        .map(|r| r["reason"].clone())
        .collect();
    assert_eq!(
        reasons,
        vec![
            json!("expired"),
            json!("expired"),
            json!("expired"),
            json!("expired"),
            json!("wrong_origin")
        ]
    );
    h.db.drop().await;
}

#[tokio::test]
async fn removing_a_passkey_steps_up_and_ends_other_sessions() {
    let Some(h) = Harness::new("pk_remove").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let mut laptop = authenticator();
    let mut phone = authenticator();
    let events = h.register(&alice, &mut laptop, "laptop").await;
    let Some(Event::PasskeyRegistered(first)) = events.first() else {
        panic!("expected PasskeyRegistered: {events:?}");
    };
    h.register(&alice, &mut phone, "phone").await;
    let other = h.sign_in("1", "alice").await;
    h.go_stale(&alice).await;
    let mut revoked = h.app.revocations.subscribe();

    let id = keel_protocol::B64Url::new(first.id.clone()).unwrap();
    let preview = match h
        .cmd(&alice, Cmd::PreviewPasskeyRemove { id: id.clone() })
        .await
        .as_slice()
    {
        [Event::Preview(p)] => p.clone(),
        other => panic!("expected Preview: {other:?}"),
    };
    assert!(preview.step_up);
    assert_eq!(preview.action, "passkey_remove");

    // No assertion: refused, preview kept.
    let events = h
        .cmd(
            &alice,
            Cmd::Confirm {
                preview_id: preview.preview_id,
                step_up: None,
            },
        )
        .await;
    assert!(
        matches!(events.as_slice(), [Event::OpError { code, .. }] if code == "step_up_required"),
        "{events:?}"
    );
    // A used ceremony: refused with the reason, preview kept.
    let used = h.assertion(&alice, &mut laptop).await;
    h.cmd(
        &alice,
        Cmd::PasskeyFinish(PasskeyResponse {
            ceremony: used.ceremony,
            credential: PasskeyCredential::Assertion(used.credential.clone()),
        }),
    )
    .await;
    h.go_stale(&alice).await;
    let events = h
        .cmd(
            &alice,
            Cmd::Confirm {
                preview_id: preview.preview_id,
                step_up: Some(used),
            },
        )
        .await;
    assert_eq!(refused(&events), PasskeyRefusedReason::Expired);

    // A fresh assertion carries it out.
    let step_up = h.assertion(&alice, &mut phone).await;
    let events = h
        .cmd(
            &alice,
            Cmd::Confirm {
                preview_id: preview.preview_id,
                step_up: Some(step_up),
            },
        )
        .await;
    match events.as_slice() {
        [Event::Confirmed { .. }, Event::Passkeys { items }] => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].name, "phone");
        }
        other => panic!("expected Confirmed then Passkeys: {other:?}"),
    }
    // The other session is over; this one carries on.
    assert_eq!(revoked.try_recv().ok(), Some(alice.user.id));
    assert_eq!(
        h.try_cmd(&other, Cmd::GetMe).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(matches!(
        h.cmd(&alice, Cmd::GetMe).await.as_slice(),
        [Event::Me(_)]
    ));
    let rows = h.audited("passkey_removed").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["target"]["name"], "laptop");

    // The last one stays.
    let last =
        keel_protocol::B64Url::new(match h.cmd(&alice, Cmd::ListPasskeys).await.as_slice() {
            [Event::Passkeys { items }] => items[0].id.clone(),
            other => panic!("{other:?}"),
        })
        .unwrap();
    let events = h.cmd(&alice, Cmd::PreviewPasskeyRemove { id: last }).await;
    assert_eq!(refused(&events), PasskeyRefusedReason::LastPasskey);
    // Someone else's or unknown: refused the same way as unknown.
    let events = h.cmd(&alice, Cmd::PreviewPasskeyRemove { id }).await;
    assert_eq!(refused(&events), PasskeyRefusedReason::UnknownCredential);
    h.db.drop().await;
}

#[tokio::test]
async fn confirm_without_a_passkey_asks_for_enrolment() {
    let Some(h) = Harness::new("pk_confirm_enrol").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let mut auth = authenticator();
    h.register(&alice, &mut auth, "laptop").await;
    // Make a step-up preview, then lose every passkey behind the session's back.
    let list = h.cmd(&alice, Cmd::ListPasskeys).await;
    let [Event::Passkeys { items }] = list.as_slice() else {
        panic!("{list:?}");
    };
    let mut phone = authenticator();
    h.register(&alice, &mut phone, "phone").await;
    let id = keel_protocol::B64Url::new(items[0].id.clone()).unwrap();
    let [Event::Preview(p)] =
        <[Event; 1]>::try_from(h.cmd(&alice, Cmd::PreviewPasskeyRemove { id }).await).unwrap()
    else {
        panic!("expected Preview");
    };
    sqlx::query("delete from passkeys")
        .execute(&h.db.owner)
        .await
        .unwrap();
    h.go_stale(&alice).await;
    let events = h
        .cmd(
            &alice,
            Cmd::Confirm {
                preview_id: p.preview_id,
                step_up: None,
            },
        )
        .await;
    assert_eq!(events, vec![Event::PasskeyEnrolmentRequired]);
    h.db.drop().await;
}

#[tokio::test]
async fn a_recovery_code_resets_passkeys_once() {
    let Some(h) = Harness::new("pk_recover").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let mut lost = authenticator();
    h.register(&alice, &mut lost, "lost").await;
    let other = h.sign_in("1", "alice").await;
    h.go_stale(&alice).await;
    let store = PgStore::new(h.db.app.clone());
    let issued = web_passkeys::recovery::issue(
        &store,
        &alice.user.id.to_string(),
        h.clock.now().unix_timestamp(),
    )
    .await
    .expect("issue");

    let wrong = RecoveryCode::new("ZZZZZZZZZZ").unwrap();
    let events = h.cmd(&alice, Cmd::RecoverPasskeys { code: wrong }).await;
    assert_eq!(refused(&events), PasskeyRefusedReason::RecoveryInvalid);

    let mut revoked = h.app.revocations.subscribe();
    let code = RecoveryCode::new(issued.code.clone()).unwrap();
    let events = h
        .cmd(&alice, Cmd::RecoverPasskeys { code: code.clone() })
        .await;
    let [Event::Verification(v)] = events.as_slice() else {
        panic!("expected Verification: {events:?}");
    };
    assert!(v.recovering && !v.enrolled && !v.verified);
    assert_eq!(revoked.try_recv().ok(), Some(alice.user.id));
    assert_eq!(
        h.try_cmd(&other, Cmd::GetMe).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Used once only.
    let events = h.cmd(&alice, Cmd::RecoverPasskeys { code }).await;
    assert_eq!(refused(&events), PasskeyRefusedReason::RecoveryInvalid);

    // The recovering session enrols a new passkey without a check.
    let mut fresh = authenticator();
    let events = h.register(&alice, &mut fresh, "new").await;
    let [Event::PasskeyRegistered(_), Event::Verification(v)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert!(v.verified && v.enrolled && !v.recovering);

    assert_eq!(h.audited("passkey_recovered").await.len(), 1);
    let denied: Vec<Value> = h
        .audited("recover_passkeys")
        .await
        .into_iter()
        .map(|r| r["reason"].clone())
        .collect();
    assert_eq!(
        denied,
        vec![json!("recovery_invalid"), json!("recovery_invalid")]
    );
    let all = serde_json::to_string(&h.audit().await).unwrap();
    assert!(!all.contains(&issued.code), "the code is never audited");
    h.db.drop().await;
}

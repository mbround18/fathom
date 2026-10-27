//! Preview/Confirm (spec 055 T015): single-use, bound to its session, expires after 5 minutes,
//! and a `Confirm` from another session is refused.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use keel_core::ports::TestClock;
use keel_protocol::{
    AssertionCredential, AssertionResponse, B64Url, CeremonyId, PasskeyAssertion,
    PasskeyExtensions, PublicKeyType,
};
use keel_server::preview::{
    ConfirmError, Confirmation, Draft, MAX_PER_SESSION, NoStepUp, Previews, StepUpVerifier, TTL,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const WINDOW: Duration = Duration::minutes(5);

fn setup() -> (Arc<TestClock>, Previews<&'static str>) {
    let clock = Arc::new(TestClock::new(
        OffsetDateTime::UNIX_EPOCH + Duration::days(20_000),
    ));
    let previews = Previews::new(clock.clone());
    (clock, previews)
}

fn draft(step_up: bool) -> Draft {
    Draft {
        action: "workspace_create".into(),
        summary: "Create workspace infra".into(),
        affected: vec!["workspace infra".into()],
        step_up,
    }
}

fn confirm<'a>(session: &'a str, p: &keel_protocol::Preview) -> Confirmation<'a> {
    Confirmation {
        session,
        preview_id: p.preview_id,
        step_up: None,
        verified_at: None,
        window: WINDOW,
    }
}

fn assertion() -> PasskeyAssertion {
    PasskeyAssertion {
        ceremony: CeremonyId(Uuid::from_u128(7)),
        credential: AssertionCredential {
            id: B64Url::new("Y3JlZA").unwrap(),
            raw_id: B64Url::new("Y3JlZA").unwrap(),
            response: AssertionResponse {
                authenticator_data: B64Url::new("YXV0aA").unwrap(),
                client_data_json: B64Url::new("Y2xpZW50").unwrap(),
                signature: B64Url::new("c2ln").unwrap(),
                user_handle: None,
            },
            kind: PublicKeyType::PublicKey,
            extensions: PasskeyExtensions::default(),
        },
    }
}

struct AcceptAll;
impl StepUpVerifier for AcceptAll {
    fn verify<'a>(
        &'a self,
        _: &'a str,
        _: &'a PasskeyAssertion,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn single_use() {
    let (_, previews) = setup();
    let p = previews.create("s1", draft(false), "op");
    assert_eq!(
        previews.confirm(confirm("s1", &p), &NoStepUp).await,
        Ok("op")
    );
    assert_eq!(
        previews.confirm(confirm("s1", &p), &NoStepUp).await,
        Err(ConfirmError::Unknown)
    );
}

#[tokio::test]
async fn another_session_is_refused_and_does_not_consume() {
    let (_, previews) = setup();
    let p = previews.create("s1", draft(false), "op");
    let err = previews
        .confirm(confirm("s2", &p), &NoStepUp)
        .await
        .unwrap_err();
    assert_eq!(err, ConfirmError::Unknown);
    assert_eq!(err.code(), "preview_unknown");
    assert_eq!(
        previews.confirm(confirm("s1", &p), &NoStepUp).await,
        Ok("op")
    );
}

#[tokio::test]
async fn expires_after_five_minutes() {
    let (clock, previews) = setup();
    let p = previews.create("s1", draft(false), "op");
    clock.advance(TTL - Duration::seconds(1));
    let q = previews.create("s1", draft(false), "op2");
    assert_eq!(
        previews.confirm(confirm("s1", &q), &NoStepUp).await,
        Ok("op2")
    );
    assert_eq!(previews.len(), 1);
    clock.advance(Duration::seconds(1));
    let err = previews
        .confirm(confirm("s1", &p), &NoStepUp)
        .await
        .unwrap_err();
    assert_eq!(
        (err.clone(), err.code()),
        (ConfirmError::Expired, "preview_expired")
    );
    assert!(previews.is_empty());
}

#[tokio::test]
async fn expires_at_is_five_minutes_out() {
    let (clock, previews) = setup();
    let p = previews.create("s1", draft(true), "op");
    let at = OffsetDateTime::parse(
        &p.expires_at,
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert_eq!(at, clock_now(&clock) + TTL);
    assert!(p.step_up);
    assert_eq!(p.action, "workspace_create");
}

fn clock_now(c: &TestClock) -> OffsetDateTime {
    keel_core::ports::Clock::now(c)
}

#[tokio::test]
async fn step_up_required_then_failed_then_ok() {
    let (clock, previews) = setup();
    let p = previews.create("s1", draft(true), "op");

    let err = previews
        .confirm(confirm("s1", &p), &NoStepUp)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "step_up_required");

    let a = assertion();
    let mut with = confirm("s1", &p);
    with.step_up = Some(&a);
    assert_eq!(
        previews.confirm(with, &NoStepUp).await,
        Err(ConfirmError::StepUpFailed)
    );
    // Neither consumed the preview.
    assert_eq!(previews.confirm(with, &AcceptAll).await, Ok("op"));

    // A session verified within the window needs no assertion.
    let q = previews.create("s1", draft(true), "op2");
    let mut recent = confirm("s1", &q);
    recent.verified_at = Some(clock_now(&clock) - Duration::minutes(4));
    assert_eq!(previews.confirm(recent, &NoStepUp).await, Ok("op2"));

    // One verified outside the window does.
    let r = previews.create("s1", draft(true), "op3");
    let mut stale = confirm("s1", &r);
    stale.verified_at = Some(clock_now(&clock) - Duration::minutes(6));
    assert_eq!(
        previews.confirm(stale, &NoStepUp).await,
        Err(ConfirmError::StepUpRequired)
    );
}

#[tokio::test]
async fn forget_session_and_per_session_cap() {
    let (_, previews) = setup();
    let first = previews.create("s1", draft(false), "first");
    for _ in 1..=MAX_PER_SESSION {
        previews.create("s1", draft(false), "more");
    }
    let other = previews.create("s2", draft(false), "other");
    assert_eq!(previews.len(), MAX_PER_SESSION + 1);
    assert_eq!(
        previews.confirm(confirm("s1", &first), &NoStepUp).await,
        Err(ConfirmError::Unknown)
    );
    previews.forget_session("s1");
    assert_eq!(previews.len(), 1);
    assert_eq!(
        previews.confirm(confirm("s2", &other), &NoStepUp).await,
        Ok("other")
    );
}

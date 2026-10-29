//! Spec 062 quickstart §3 ("End to end on the dev compose"), against a real daemon: Depths driven
//! over its WebSocket by an administrator with a test passkey, a local `distribution` registry,
//! and the real check scheduler and Auto queue running as `main` starts them.
//!
//! Opt-in: `DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_walkthrough`. Without
//! `DEPTHS_IT_DOCKER=1` every test returns at once. Run by the main session only.
//!
//! Everything a test creates is named `depths-it-062-<random>…` and removed by name when its
//! [`Lab`] drops, also when an assertion fails. Nothing else on the daemon is touched. The tests run
//! one at a time: Watchtower detection reads every container on the daemon, so one test's
//! Watchtower would pause another's Auto queue.
//!
//! Waits are on what is awaited, never on the first event of a kind: [`view_where`] reads views
//! until one satisfies its predicate, and asks for a fresh one when the socket goes quiet
//! (`UpdatesChanged` is coalesced). No test sleeps for a maintenance window: each Auto stack's
//! window is the two hours around now.

mod support;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use depths_protocol::{
    AuditEntry, AutoPause, Exclusion, Provider, RunTrigger, ServiceUpdate, StackOutcome, UpdateRun,
    UpdateState, UpdatesView,
};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use support::OWNER;
use support::docker::*;
use support::passkey::{Authenticator, answer, authenticator, enrol, open_unverified, stale};
use support::ws::{Socket, ask};
use tokio_tungstenite::tungstenite::Message;

const ROOT: &str = "depths-it-062";
/// Check now may be asked again after a second, so a test can ask twice.
const ENV: &[(&str, &str)] = &[("DEPTHS_UPDATES_CHECK_GAP", "1s")];
/// A check pass against the local registry, or an update run with its health wait.
const RUN: Duration = Duration::from_secs(120);
const WATCHTOWER: &str = "containrrr/watchtower:1.7.1";

/// One test at a time (see the module doc).
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cmd(ty: &str, data: Value) -> Value {
    json!({ "type": ty, "data": data })
}

fn compose(image: &str) -> String {
    format!("services:\n  app:\n    image: {image}\n")
}

/// `app` from the lab's registry, plus `side`, which no test updates.
fn two_services(lab: &Lab) -> String {
    format!(
        "services:\n  app:\n    image: {}\n  side:\n    image: {}\n",
        lab.reference("app", "1"),
        lab.reference("side", "1")
    )
}

/// Pushes a new `app:1` built from `dockerfile` and points the local tag back at the image the
/// stack runs, as on a host that has not pulled yet. Returns the new image's id.
fn repush(lab: &Lab, dockerfile: &str) -> String {
    let reference = lab.reference("app", "1");
    let old = image_id(&reference).expect("app:1 was pushed before");
    let new = lab.push("app", "1", dockerfile);
    ok(&["tag", &old, &reference]);
    new
}

/// An image whose command fails at once, with no healthcheck: its container exits. It keeps
/// a non-root `USER`, or Depths would hold it for turning root (SC-005) instead of running it.
fn failing(version: &str) -> String {
    format!(
        "FROM busybox:1.37\n\
         LABEL org.opencontainers.image.version={version}\n\
         USER 1000\n\
         CMD [\"false\"]\n"
    )
}

/// An image whose healthcheck stays `starting` for an hour: an update parks in its health wait.
fn slow_to_start(version: &str) -> String {
    format!(
        "FROM busybox:1.37\n\
         LABEL org.opencontainers.image.version={version}\n\
         USER 1000\n\
         HEALTHCHECK --interval=1h --start-period=1h CMD true\n\
         CMD [\"sleep\", \"86400\"]\n"
    )
}

/// The window from an hour ago to an hour from now, every day, in host local time (the zone
/// `auto::HostZone` reads).
fn window_around_now() -> Value {
    let now = chrono::Local::now();
    let at = |t: chrono::DateTime<chrono::Local>| t.format("%H:%M").to_string();
    json!({
        "days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
        "start": at(now - chrono::Duration::hours(1)),
        "end": at(now + chrono::Duration::hours(1)),
    })
}

/// `docker inspect` of a container: `(Config.Image, Image)`.
fn running_image(container: &str) -> (String, String) {
    let out = ok(&[
        "inspect",
        "--format",
        "{{.Config.Image}} {{.Image}}",
        container,
    ]);
    let (reference, id) = out.split_once(' ').expect("two fields");
    (reference.to_owned(), id.to_owned())
}

fn service<'a>(v: &'a UpdatesView, stack: &str, name: &str) -> Option<&'a ServiceUpdate> {
    v.services
        .iter()
        .find(|s| s.stack == stack && s.service == name)
}

fn state(v: &UpdatesView, stack: &str) -> Option<UpdateState> {
    service(v, stack, "app").map(|s| s.state)
}

/// The latest run that names `stack`, and that stack's part of it.
fn run_of<'a>(v: &'a UpdatesView, stack: &str) -> Option<(&'a UpdateRun, StackOutcome)> {
    v.recent_runs.iter().rev().find_map(|r| {
        r.stacks
            .iter()
            .find(|s| s.stack == stack)
            .and_then(|s| s.outcome.clone())
            .map(|o| (r, o))
    })
}

fn view_of(e: &Value) -> Option<UpdatesView> {
    matches!(
        e["event"]["type"].as_str(),
        Some("Updates" | "UpdatesChanged")
    )
    .then(|| serde_json::from_value(e["event"]["data"].clone()).expect("an UpdatesView"))
}

/// The next text event within `within`, or `None`.
async fn recv(ws: &mut Socket, within: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                return Some(serde_json::from_str(t.as_str()).unwrap());
            }
            Ok(Some(Ok(_))) => {}
            Ok(other) => panic!("the socket closed: {other:?}"),
            Err(_) => return None,
        }
    }
}

/// Request ids for `ListUpdates` asked while waiting, clear of the ones tests pick.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(70_000);

/// Reads update views until one satisfies `pred`. When the socket is quiet for two seconds it asks
/// `ListUpdates` again, so a coalesced `UpdatesChanged` cannot be missed.
async fn view_where(
    ws: &mut Socket,
    within: Duration,
    what: &str,
    pred: impl Fn(&UpdatesView) -> bool,
) -> UpdatesView {
    let deadline = tokio::time::Instant::now() + within;
    let mut last = None;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!left.is_zero(), "timed out: {what}; last view: {last:#?}");
        match recv(ws, left.min(Duration::from_secs(2))).await {
            Some(e) => {
                if let Some(v) = view_of(&e) {
                    if pred(&v) {
                        return v;
                    }
                    last = Some(v);
                }
            }
            None => {
                let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let ask = json!({ "id": id, "cmd": { "type": "ListUpdates" } });
                ws.send(Message::text(ask.to_string())).await.unwrap();
            }
        }
    }
}

/// Reads events until one satisfies `pred`.
async fn event_where(
    ws: &mut Socket,
    within: Duration,
    what: &str,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let e = recv(ws, left)
            .await
            .unwrap_or_else(|| panic!("timed out: {what}"));
        if pred(&e) {
            return e;
        }
    }
}

/// Returns once the wall clock is past the second `t` (RFC 3339, to the second) names.
async fn past_second(t: &Option<String>) {
    let t = t.as_deref().expect("checked once");
    let t = chrono::DateTime::parse_from_rfc3339(t).unwrap() + chrono::Duration::seconds(1);
    while chrono::Utc::now() < t {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// An administrator's socket, signed in with a passkey enrolled now (so a fresh check).
async fn admin(lab: &Lab) -> (Socket, tokio::task::JoinHandle<()>, String, Authenticator) {
    let (mut ws, server, cookie) = open_unverified(&lab.app, OWNER).await;
    let mut auth = authenticator();
    enrol(&mut ws, &mut auth, "Laptop").await;
    (ws, server, cookie, auth)
}

/// `ListUpdates`: the view, and live changes from now on.
async fn subscribe(ws: &mut Socket) -> UpdatesView {
    let r = ask(ws, 1, json!({ "type": "ListUpdates" })).await;
    view_of(&r).unwrap_or_else(|| panic!("no view: {r}"))
}

async fn check_now(ws: &mut Socket, id: u64, stacks: Option<&[&str]>) {
    // `stacks` is left out for "every stack": the protocol does not take `null`.
    let body = match stacks {
        Some(stacks) => json!({ "stacks": stacks }),
        None => json!({}),
    };
    let r = ask(ws, id, cmd("CheckUpdatesNow", body)).await;
    assert_eq!(r["event"]["type"], "CheckStarted", "{r}");
}

/// Confirms the preview in `r`, answering its passkey check when it asks for one.
async fn confirm(ws: &mut Socket, id: u64, auth: &mut Authenticator, r: &Value) -> Value {
    let data = &r["event"]["data"];
    let mut confirm = json!({ "preview_id": data["preview_id"] });
    if data["step_up"].is_object() {
        confirm["assertion"] = answer(auth, support::ORIGIN, &data["step_up"]);
    }
    if data["stacks"]
        .as_array()
        .is_some_and(|s| s.iter().any(|p| p["ack_required"] == true))
    {
        confirm["acknowledge_findings"] = json!(true);
    }
    ask(ws, id, cmd("Confirm", confirm)).await
}

/// Previews and confirms an update change (`PreviewUpdateChange`).
async fn change(ws: &mut Socket, id: u64, auth: &mut Authenticator, change: Value) {
    let r = ask(ws, id, cmd("PreviewUpdateChange", change)).await;
    assert_eq!(r["event"]["type"], "Preview", "{r}");
    let r = confirm(ws, id + 1, auth, &r).await;
    assert_eq!(r["event"]["type"], "UpdatesChanged", "{r}");
}

/// Sets `stack` to Auto in the window around now.
async fn auto_now(ws: &mut Socket, id: u64, auth: &mut Authenticator, stack: &str) {
    let policy = json!({ "stack": stack, "policy": "auto", "window": window_around_now() });
    change(ws, id, auth, json!({ "kind": "policy", "data": policy })).await;
}

fn audit_of(lab: &Lab, stack: &str) -> Vec<AuditEntry> {
    lab.app
        .audit_entries()
        .into_iter()
        .filter(|e| e.stack.as_deref() == Some(stack))
        .collect()
}

fn scheduled_for_owner(trigger: &RunTrigger, stack: &str) -> bool {
    matches!(trigger, RunTrigger::Scheduled { on_behalf_of }
        if on_behalf_of.get(stack).is_some_and(|a| a.subject == OWNER))
}

// --- §3 step 1 -----------------------------------------------------------------------------------

/// The shipped compose file gives Depths the host's zone, so windows are in host local time.
#[test]
fn the_dev_compose_mounts_the_host_localtime_into_depths() {
    if !enabled() {
        return;
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../compose.yaml");
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let volumes = doc["services"]["depths"]["volumes"]
        .as_sequence()
        .expect("depths has volumes");
    assert!(
        volumes
            .iter()
            .any(|v| v.as_str() == Some("/etc/localtime:/etc/localtime:ro")),
        "{volumes:?}"
    );
}

// --- §3 step 2 -----------------------------------------------------------------------------------

/// Turning checks on needs the passkey on a stale session; Check now then lists every stack, and
/// a stack pinned to a digest is shown excluded.
#[test]
fn turning_checks_on_steps_up_and_check_now_lists_every_stack_with_a_digest_pin_excluded() {
    if !enabled() {
        return;
    }
    let _one = serial();
    let mut lab = Lab::with(ROOT, ENV);
    lab.u()
        .store()
        .update_decisions(|d| d.settings.checks_enabled = false)
        .unwrap();
    lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    let reference = lab.reference("app", "1");
    let repo = reference.trim_end_matches(":1").to_owned();
    let digests = ok(&[
        "image",
        "inspect",
        "--format",
        "{{json .RepoDigests}}",
        &reference,
    ]);
    let digests: Vec<String> = serde_json::from_str(&digests).unwrap();
    let pinned = digests
        .into_iter()
        .find(|d| d.starts_with(&format!("{repo}@")))
        .expect("the push recorded a digest");
    let tagged = lab.stack("tagged", &compose(&reference));
    let pin = lab.stack("pinned", &compose(&pinned));
    lab.start_tasks();

    lab.rt.block_on(async {
        let (mut ws, server, cookie, mut auth) = admin(&lab).await;
        stale(&lab.app, &cookie);
        let mut settings = serde_json::to_value(lab.u().store().settings()).unwrap();
        settings["checks_enabled"] = json!(true);
        let r = ask(
            &mut ws,
            2,
            cmd(
                "PreviewUpdateChange",
                json!({ "kind": "settings", "data": settings }),
            ),
        )
        .await;
        assert_eq!(r["event"]["type"], "Preview", "{r}");
        let request = r["event"]["data"]["step_up"].clone();
        assert!(
            request.is_object(),
            "turning checks on asks for the passkey: {r}"
        );
        let preview_id = r["event"]["data"]["preview_id"].clone();
        let refused = ask(
            &mut ws,
            3,
            cmd("Confirm", json!({ "preview_id": preview_id })),
        )
        .await;
        assert_eq!(refused["event"]["type"], "StepUpRequired", "{refused}");
        assert!(
            !lab.u().checks_enabled(),
            "nothing changed without the passkey"
        );
        let assertion = answer(&mut auth, support::ORIGIN, &request);
        let r = ask(
            &mut ws,
            4,
            cmd(
                "Confirm",
                json!({ "preview_id": preview_id, "assertion": assertion }),
            ),
        )
        .await;
        assert_eq!(r["event"]["type"], "UpdatesChanged", "{r}");
        assert!(lab.u().checks_enabled());

        // Turning checks on wakes the scheduler, whose first pass checks every stack. Check now
        // must then check them again: its view has a later `last_checked`.
        subscribe(&mut ws).await;
        let first = view_where(&mut ws, RUN, "the first pass", |v| {
            state(v, &tagged) == Some(UpdateState::UpToDate)
        })
        .await;
        let checked = service(&first, &tagged, "app")
            .unwrap()
            .last_checked
            .clone();
        // `last_checked` is to the second: Check now's pass must fall in a later one.
        past_second(&checked).await;
        check_now(&mut ws, 5, None).await;
        let v = view_where(&mut ws, RUN, "Check now's pass", |v| {
            service(v, &tagged, "app").is_some_and(|s| s.last_checked > checked)
        })
        .await;
        let names: Vec<(&str, &str)> = v
            .services
            .iter()
            .map(|s| (s.stack.as_str(), s.service.as_str()))
            .collect();
        assert!(names.contains(&(tagged.as_str(), "app")), "{names:?}");
        assert!(names.contains(&(pin.as_str(), "app")), "{names:?}");
        let excluded = service(&v, &pin, "app").unwrap();
        assert_eq!(
            excluded.effective.excluded,
            Some(Exclusion::PinnedByDigest),
            "{excluded:#?}"
        );
        assert_eq!(state(&v, &tagged), Some(UpdateState::UpToDate));
        server.abort();
    });
}

// --- §3 step 3 -----------------------------------------------------------------------------------

/// A new `app:1` in the registry shows as an update, with the version label of the image
/// running and of the one available.
#[test]
fn a_new_image_behind_the_same_tag_shows_as_an_update_with_its_version_label() {
    if !enabled() {
        return;
    }
    let _one = serial();
    let mut lab = Lab::with(ROOT, ENV);
    lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    let stack = lab.stack("label", &compose(&lab.reference("app", "1")));
    lab.start_tasks();

    lab.rt.block_on(async {
        let (mut ws, server, _, _) = admin(&lab).await;
        subscribe(&mut ws).await;
        view_where(&mut ws, RUN, "the first pass", |v| {
            state(v, &stack) == Some(UpdateState::UpToDate)
        })
        .await;
        repush(&lab, &dockerfile("1.1.0", "1000", true));
        check_now(&mut ws, 2, Some(&[stack.as_str()])).await;
        let v = view_where(&mut ws, RUN, "the update shows", |v| {
            state(v, &stack) == Some(UpdateState::Available)
        })
        .await;
        let s = service(&v, &stack, "app").unwrap();
        assert_eq!(
            s.available.as_ref().and_then(|a| a.version.as_deref()),
            Some("1.1.0"),
            "{s:#?}"
        );
        assert_eq!(
            s.running.as_ref().and_then(|r| r.version.as_deref()),
            Some("1.0.0"),
            "{s:#?}"
        );
        server.abort();
    });
}

// --- §3 step 4 -----------------------------------------------------------------------------------

/// A batch of a stack with an update and one without: the preview lists the one service, and the
/// confirmed run recreates only it.
#[test]
fn a_batch_with_an_up_to_date_stack_previews_and_recreates_only_the_changed_service() {
    if !enabled() {
        return;
    }
    let _one = serial();
    let mut lab = Lab::with(ROOT, ENV);
    lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    lab.push("side", "1", &dockerfile("1.0.0", "1000", true));
    let changed = lab.stack("changed", &two_services(&lab));
    let current = lab.stack("current", &compose(&lab.reference("side", "1")));
    lab.start_tasks();
    let before = (
        lab.container(&changed, "app"),
        lab.container(&changed, "side"),
        lab.container(&current, "app"),
    );

    lab.rt.block_on(async {
        let (mut ws, server, _, mut auth) = admin(&lab).await;
        subscribe(&mut ws).await;
        view_where(&mut ws, RUN, "the first pass", |v| {
            state(v, &changed) == Some(UpdateState::UpToDate)
                && state(v, &current) == Some(UpdateState::UpToDate)
        })
        .await;
        let new = repush(&lab, &dockerfile("1.1.0", "1000", true));
        check_now(&mut ws, 2, Some(&[changed.as_str()])).await;
        view_where(&mut ws, RUN, "the update shows", |v| {
            state(v, &changed) == Some(UpdateState::Available)
        })
        .await;

        let r = ask(
            &mut ws,
            3,
            cmd(
                "PreviewUpdateBatch",
                json!({ "stacks": [changed, current] }),
            ),
        )
        .await;
        assert_eq!(r["event"]["type"], "UpdatePreview", "{r}");
        let plans = r["event"]["data"]["stacks"].as_array().unwrap();
        let services = |i: usize| -> Vec<String> {
            plans[i]["services"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["service"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(plans[0]["stack"], changed.as_str(), "{r}");
        assert_eq!(services(0), ["app"], "one service of the batch: {r}");
        assert!(services(1).is_empty(), "the up-to-date stack has none: {r}");

        let started = confirm(&mut ws, 4, &mut auth, &r).await;
        assert_eq!(started["event"]["type"], "UpdateRunStarted", "{started}");
        let run_id = started["event"]["data"]["run_id"].clone();
        let done = event_where(&mut ws, RUN, "the run finishes", |e| {
            e["event"]["type"] == "UpdateRunFinished" && e["event"]["data"]["run_id"] == run_id
        })
        .await;
        let summary: Vec<(String, StackOutcome)> =
            serde_json::from_value(done["event"]["data"]["summary"].clone()).unwrap();
        assert!(
            summary.contains(&(changed.clone(), StackOutcome::Updated)),
            "{summary:?}"
        );
        assert!(
            !summary.contains(&(current.clone(), StackOutcome::Updated)),
            "{summary:?}"
        );

        let app = lab.container(&changed, "app");
        assert_ne!(app, before.0, "the changed service was recreated");
        assert_eq!(running_image(&app).1, new);
        assert_eq!(
            lab.container(&changed, "side"),
            before.1,
            "its sibling was not"
        );
        assert_eq!(
            lab.container(&current, "app"),
            before.2,
            "the other stack was not"
        );
        server.abort();
    });
    lab.release();
}

// --- §3 step 5 -----------------------------------------------------------------------------------

/// An Auto stack in its window gets an image whose command fails: the scheduled run rolls it back,
/// and the audit names "Depths (scheduled)" on behalf of the administrator who set Auto.
#[test]
fn a_failing_image_on_an_auto_stack_is_rolled_back_by_depths_on_behalf_of_its_author() {
    if !enabled() {
        return;
    }
    let _one = serial();
    let mut lab = Lab::with(ROOT, ENV);
    let old = lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    let stack = lab.stack("auto", &compose(&lab.reference("app", "1")));
    lab.start_tasks();

    lab.rt.block_on(async {
        let (mut ws, server, _, mut auth) = admin(&lab).await;
        subscribe(&mut ws).await;
        view_where(&mut ws, RUN, "the first pass", |v| {
            state(v, &stack) == Some(UpdateState::UpToDate)
        })
        .await;
        auto_now(&mut ws, 2, &mut auth, &stack).await;
        repush(&lab, &failing("2.0.0"));
        check_now(&mut ws, 4, Some(&[stack.as_str()])).await;
        let v = view_where(&mut ws, RUN, "the scheduled run rolls back", |v| {
            run_of(v, &stack).is_some_and(|(_, o)| o == StackOutcome::RolledBack)
        })
        .await;
        let (run, _) = run_of(&v, &stack).unwrap();
        assert!(scheduled_for_owner(&run.trigger, &stack), "{run:#?}");
        server.abort();
    });

    let by_depths: Vec<AuditEntry> = audit_of(&lab, &stack)
        .into_iter()
        .filter(|e| e.actor.provider == Provider::Depths)
        .collect();
    assert!(!by_depths.is_empty(), "{:#?}", audit_of(&lab, &stack));
    for e in &by_depths {
        assert_eq!(e.actor.username, "Depths (scheduled)", "{e:#?}");
        assert_eq!(
            e.on_behalf_of.as_ref().map(|a| a.subject.as_str()),
            Some(OWNER),
            "{e:#?}"
        );
    }
    let (_, image) = running_image(&lab.container(&stack, "app"));
    assert_eq!(image, old, "the stack runs the image it ran before");
    lab.release();
}

// --- §3 step 6 -----------------------------------------------------------------------------------

/// Removes the Watchtower image on drop when the test pulled it.
struct Pulled(Option<&'static str>);

impl Drop for Pulled {
    fn drop(&mut self) {
        if let Some(image) = self.0 {
            let _ = docker(&["image", "rm", image]);
        }
    }
}

/// A Watchtower (monitor-only) beside Depths shows the banner and pauses Auto; once it is
/// acknowledged, the queue runs the update it held back.
#[test]
fn a_watchtower_beside_depths_pauses_auto_until_it_is_acknowledged() {
    if !enabled() {
        return;
    }
    let _one = serial();
    let _pulled = Pulled(image_id(WATCHTOWER).is_none().then_some(WATCHTOWER));
    let mut lab = Lab::with(ROOT, ENV);
    lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    let stack = lab.stack("watchtower", &compose(&lab.reference("app", "1")));
    let before = lab.container(&stack, "app");
    lab.start_tasks();
    // Monitor-only, scoped to a scope nothing has, polling once a day: it touches no container.
    // 1.7.1 speaks Docker API 1.40 at most; the daemon accepts it when asked for.
    let name = format!("{}-watchtower", lab.prefix);
    lab.containers.lock().unwrap().push(name.clone());
    let scope = format!("WATCHTOWER_SCOPE={}", lab.prefix);
    ok(&[
        "run",
        "-d",
        "--name",
        &name,
        "-v",
        "/var/run/docker.sock:/var/run/docker.sock:ro",
        "-e",
        "DOCKER_API_VERSION=1.40",
        "-e",
        "WATCHTOWER_MONITOR_ONLY=true",
        "-e",
        &scope,
        "-e",
        "WATCHTOWER_POLL_INTERVAL=86400",
        WATCHTOWER,
    ]);

    lab.rt.block_on(async {
        let (mut ws, server, _, mut auth) = admin(&lab).await;
        subscribe(&mut ws).await;
        view_where(&mut ws, RUN, "the first pass", |v| {
            state(v, &stack) == Some(UpdateState::UpToDate)
        })
        .await;
        auto_now(&mut ws, 2, &mut auth, &stack).await;
        let new = repush(&lab, &dockerfile("1.1.0", "1000", true));
        check_now(&mut ws, 4, Some(&[stack.as_str()])).await;
        let v = view_where(&mut ws, RUN, "the banner and the update", |v| {
            state(v, &stack) == Some(UpdateState::Available)
                && v.watchtower.iter().any(|w| w.name == name)
                && matches!(&v.auto_paused, Some(AutoPause::WatchtowerRunning { containers })
                    if containers.contains(&name))
        })
        .await;
        let found = v.watchtower.iter().find(|w| w.name == name).unwrap();
        assert_eq!(found.acknowledged, None, "{found:#?}");
        let container_id = found.id.clone();

        // An Auto pass that starts now sees the update and the pause: nudge one, wait for it to
        // finish, and it has run nothing. Passes run one after another, so the first to finish
        // past `since` started after the read.
        let mut passes = lab.u().auto_passes();
        let since = passes.borrow().started;
        lab.u().notify_changed();
        let pass = *tokio::time::timeout(RUN, passes.wait_for(|p| p.finished > since))
            .await
            .expect("an Auto pass finished")
            .unwrap();
        assert_eq!(pass.last_ran, 0, "{pass:?}");
        let r = ask(&mut ws, 5, json!({ "type": "ListUpdates" })).await;
        let v = view_of(&r).unwrap();
        assert!(v.auto_paused.is_some(), "{:#?}", v.auto_paused);
        assert!(run_of(&v, &stack).is_none(), "{:#?}", v.recent_runs);
        assert_eq!(
            lab.container(&stack, "app"),
            before,
            "nothing was recreated"
        );

        let ack = json!({ "container_id": container_id, "answer": "monitor_only" });
        change(
            &mut ws,
            6,
            &mut auth,
            json!({ "kind": "ack_watchtower", "data": ack }),
        )
        .await;
        let v = view_where(&mut ws, RUN, "Auto runs once acknowledged", |v| {
            v.auto_paused.is_none()
                && run_of(v, &stack).is_some_and(|(_, o)| o == StackOutcome::Updated)
        })
        .await;
        let (run, _) = run_of(&v, &stack).unwrap();
        assert!(scheduled_for_owner(&run.trigger, &stack), "{run:#?}");
        let app = lab.container(&stack, "app");
        assert_ne!(app, before);
        assert_eq!(running_image(&app).1, new);
        server.abort();
    });
    lab.release();
}

// --- §3 step 7 -----------------------------------------------------------------------------------

/// Depths stops while a scheduled update waits for health. Started again on the same data, it
/// records the run interrupted, with the image the stack really runs, and checks that image.
///
/// The real Auto queue runs it (`auto::run`, started as `main` does), with a health-wait sleep
/// that parks for good, standing in for the process dying there.
#[test]
fn a_restart_during_the_health_wait_records_the_run_interrupted_with_the_real_image() {
    if !enabled() {
        return;
    }
    let _one = serial();
    let mut lab = Lab::with(ROOT, ENV);
    lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    let stack = lab.stack("restart", &compose(&lab.reference("app", "1")));
    lab.rt.block_on(async {
        let (mut ws, server, _, mut auth) = admin(&lab).await;
        auto_now(&mut ws, 2, &mut auth, &stack).await;
        server.abort();
    });
    let new = repush(&lab, &slow_to_start("2.0.0"));
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);

    let (waiting, parked) = std::sync::mpsc::sync_channel::<()>(1);
    let waiting = Mutex::new(waiting);
    // The "old process": the Auto tick parks in its health wait until the test binary exits.
    lab.start_tasks_with(Arc::new(move |_: Duration| {
        let _ = waiting.lock().unwrap().try_send(());
        loop {
            std::thread::park();
        }
    }));
    parked
        .recv_timeout(RUN)
        .expect("the scheduled run reached its health wait");
    let open: Vec<UpdateRun> = lab.u().store().observed().open_runs().cloned().collect();
    assert_eq!(open.len(), 1, "{open:#?}");
    assert_eq!(
        open[0].stacks[0].phase,
        depths_protocol::RunPhase::Waiting,
        "{open:#?}"
    );

    assert_eq!(lab.restart(), 1, "the start-up pass closed the open run");
    let container = lab.container(&stack, "app");
    let (reference, image) = running_image(&container);
    assert_eq!(image, new, "the new container is what runs");

    let interrupted: Vec<AuditEntry> = audit_of(&lab, &stack)
        .into_iter()
        .filter(|e| serde_json::to_value(e.action).unwrap() == "update_interrupted")
        .collect();
    assert_eq!(interrupted.len(), 1, "{interrupted:#?}");
    let e = &interrupted[0];
    assert_eq!(e.actor.username, "Depths (scheduled)", "{e:#?}");
    assert_eq!(
        e.on_behalf_of.as_ref().map(|a| a.subject.as_str()),
        Some(OWNER)
    );
    assert_eq!(
        e.diff_summary.as_deref(),
        Some(format!("running now: app={reference}").as_str()),
        "{e:#?}"
    );

    lab.check(&stack);
    lab.rt.block_on(async {
        // The passkey is enrolled already; a signed-in session is verified for now.
        let (mut ws, server, _) = support::ws::open(&lab.app, OWNER).await;
        let v = subscribe(&mut ws).await;
        let (run, outcome) = run_of(&v, &stack).expect("the run is listed");
        assert_eq!(outcome, StackOutcome::Interrupted, "{run:#?}");
        assert!(run.finished_at.is_some(), "{run:#?}");
        let s = service(&v, &stack, "app").unwrap();
        assert_eq!(
            s.running.as_ref().map(|r| r.image_id.as_str()),
            Some(new.as_str()),
            "the page shows the image the stack really runs: {s:#?}"
        );
        server.abort();
    });
    lab.release();
}

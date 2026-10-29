//! Spec 062 quickstart §3 ("End to end on the dev compose"), against a real daemon: Depths driven
//! over its WebSocket by an administrator with a test passkey, a local `distribution` registry,
//! and the real check scheduler and Auto queue running as `main` starts them.
//!
//! Opt-in: `DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_walkthrough`. Without
//! `DEPTHS_IT_DOCKER=1` every test returns at once. Run by the main session only.
//!
//! Everything a test creates is named `depths-it-062-<random>…` and removed by name when its
//! [`Lab`] drops, also when an assertion fails. Nothing else on the daemon is touched. Each test
//! has its own registry, data dir and stacks, so they run in parallel.
//!
//! Step 6 (a Watchtower beside Depths) is its own target, `update_watchtower`: Watchtower detection
//! reads every container on the daemon, so its Watchtower would pause the Auto queues here. Run
//! the two targets one after the other, never at the same time.
//!
//! Waits are on what is awaited, never on the first event of a kind: [`view_where`] reads views
//! until one satisfies its predicate, and asks for a fresh one when the socket goes quiet
//! (`UpdatesChanged` is coalesced). No test sleeps for a maintenance window: each Auto stack's
//! window is the two hours around now.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use depths_protocol::{AuditEntry, Exclusion, Provider, StackOutcome, UpdateRun, UpdateState};
use serde_json::{Value, json};
use support::OWNER;
use support::docker::*;
use support::passkey::{answer, stale};
use support::walk::*;
use support::ws::{Socket, ask};

const ROOT: &str = "depths-it-062";
/// Check now may be asked again after a second, so a test can ask twice.
const ENV: &[(&str, &str)] = &[("DEPTHS_UPDATES_CHECK_GAP", "1s")];

/// `app` from the lab's registry, plus `side`, which no test updates.
fn two_services(lab: &Lab) -> String {
    format!(
        "services:\n  app:\n    image: {}\n  side:\n    image: {}\n",
        lab.reference("app", "1"),
        lab.reference("side", "1")
    )
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

fn audit_of(lab: &Lab, stack: &str) -> Vec<AuditEntry> {
    lab.app
        .audit_entries()
        .into_iter()
        .filter(|e| e.stack.as_deref() == Some(stack))
        .collect()
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

/// The runs `stack` is in, from the journal on disk.
fn runs_of(lab: &Lab, stack: &str) -> Vec<UpdateRun> {
    lab.u()
        .store()
        .observed()
        .journal
        .into_iter()
        .filter(|r| r.stacks.iter().any(|s| s.stack == stack))
        .collect()
}

/// Auto updates a stack in its window; Depths restarts inside the same window with another
/// update waiting, and the queue does not run the stack a second time: the occurrence it ran in
/// is on disk.
#[test]
fn a_restart_inside_the_window_does_not_run_an_auto_stack_again() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::with(ROOT, ENV);
    lab.push("app", "1", &dockerfile("1.0.0", "1000", true));
    let stack = lab.stack("once", &compose(&lab.reference("app", "1")));
    lab.rt.block_on(async {
        let (mut ws, server, _, mut auth) = admin(&lab).await;
        auto_now(&mut ws, 2, &mut auth, &stack).await;
        server.abort();
    });
    let first = repush(&lab, &dockerfile("1.1.0", "1000", true));
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);

    lab.start_tasks();
    let deadline = std::time::Instant::now() + RUN;
    let ran = loop {
        let runs = runs_of(&lab, &stack);
        if runs.iter().any(|r| r.finished_at.is_some()) {
            break runs;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the Auto run finished"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(ran.len(), 1, "{ran:#?}");
    assert_eq!(
        ran[0].stacks[0].outcome,
        Some(StackOutcome::Updated),
        "{ran:#?}"
    );
    let updated = lab.container(&stack, "app");
    assert_eq!(running_image(&updated).1, first);

    // The same window, a new process, and another update the queue would install if it forgot.
    assert_eq!(lab.restart(), 0, "nothing was left open");
    repush(&lab, &dockerfile("1.2.0", "1000", true));
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);
    let mut passes = lab.u().auto_passes();
    lab.start_tasks();
    lab.rt.block_on(async {
        // A pass runs its stacks before it counts as finished, so a second run would show now.
        tokio::time::timeout(RUN, passes.wait_for(|p| p.finished >= 1))
            .await
            .expect("an Auto pass finished")
            .unwrap();
    });
    assert_eq!(
        runs_of(&lab, &stack).len(),
        1,
        "no second run in the window"
    );
    assert_eq!(
        lab.container(&stack, "app"),
        updated,
        "nothing was recreated"
    );
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);
    lab.release();
}

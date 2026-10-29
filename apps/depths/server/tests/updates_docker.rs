//! Spec 062 T049, T063, T078: image updates against a real daemon and a local `distribution`
//! registry (quickstart §2).
//!
//! Opt-in: `DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test updates_docker`. Without
//! `DEPTHS_IT_DOCKER=1` every test returns at once. Run by the main session only.
//!
//! Each test starts its own registry container and `depths-it-upd-<random>` stacks. On drop it
//! removes, by name, what it created: the stacks' containers, the registry, the images it built and
//! pushed, and the stacks' `depths-keep/…` tags. It never touches other containers, volumes or
//! images. Images build `FROM busybox:1.37`, which needs Docker Hub once.
//!
//! The harness ([`support::docker::Lab`]) is shared with `update_walkthrough`.

mod support;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use depths_core::updates::keep::{self, KeepKind};
use depths_protocol::{StackOutcome, UpdateRun, UpdateState};
use oci_registry::RequestCounts;
use support::docker::*;

/// `app` from our registry, plus `side`, which no test updates.
fn two_services(lab: &Lab) -> String {
    format!(
        "services:\n  app:\n    image: {}\n  side:\n    image: {}\n",
        lab.reference("app", "1"),
        lab.reference("side", "1")
    )
}

fn sum(calls: &[RequestCounts]) -> RequestCounts {
    calls
        .iter()
        .fold(RequestCounts::default(), |a, c| RequestCounts {
            head: a.head + c.head,
            get: a.get + c.get,
            manifest_get: a.manifest_get + c.manifest_get,
        })
}

fn outcome(run: &UpdateRun) -> Option<StackOutcome> {
    run.stacks.first().and_then(|s| s.outcome.clone())
}

// --- US2 (T049) ----------------------------------------------------------------------------------

#[test]
fn an_unchanged_tag_is_head_only_and_a_pushed_image_is_resolved_once() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("head", &two_services(&lab));
    lab.check(&stack);

    let before = lab.access_log().len();
    let calls = lab.check(&stack);
    let total = sum(&calls);
    assert!(total.head >= 1, "{calls:?}");
    assert_eq!(total.get, 0, "an unchanged tag is HEAD only: {calls:?}");
    assert_eq!(lab.state(&stack, "app"), UpdateState::UpToDate);
    let log = lab.access_log();
    assert!(
        log[before..].iter().all(|l| !l.contains("\"GET /v2/")),
        "no GET reached the registry: {:?}",
        &log[before..]
    );

    lab.push("app", "1", &dockerfile("2", "1000", true));
    let calls = lab.check(&stack);
    // The new digest is resolved once. On the containerd image store the running image's id is
    // the digest it was pulled by, so the checker also resolves that once to compare platform
    // manifests (check.rs `same_image`); both are cached from then on.
    let expected = if containerd_store() { 2 } else { 1 };
    assert_eq!(
        calls.iter().filter(|c| c.get > 0).count(),
        expected,
        "one resolution per digest never seen: {calls:?}"
    );
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);
    let calls = lab.check(&stack);
    assert_eq!(sum(&calls).get, 0, "resolved once, then cached: {calls:?}");
    let observed = lab.u().store().observed();
    let available = observed.services[&format!("{stack}/app")]
        .available
        .clone()
        .unwrap();
    assert_eq!(available.version.as_deref(), Some("2"));
    assert_eq!(lab.state(&stack, "side"), UpdateState::UpToDate);
}

#[test]
fn a_multi_arch_repush_of_the_same_image_is_not_an_update() {
    if !enabled() {
        return;
    }
    if !docker(&["buildx", "version"]).status.success() {
        eprintln!("skipped: docker buildx is not installed (imagetools makes the index)");
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("multi", &two_services(&lab));
    lab.check(&stack);
    // An index over the same host manifest: the tag's digest changes, the image does not.
    let reference = lab.reference("app", "1");
    ok(&[
        "buildx",
        "imagetools",
        "create",
        "-t",
        &reference,
        &reference,
    ]);
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::UpToDate);
    assert!(lab.nothing_to_update(&stack));
}

/// A newer image pulled by hand takes the tag, and the image the container runs is left without a
/// name (under the containerd store the daemon no longer inspects it). The service still shows
/// its container, by image id, with the new image waiting.
#[test]
fn a_hand_pulled_tag_leaves_the_running_image_unnamed_and_the_service_updatable() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("unnamed", &two_services(&lab));
    let old = image_id(&lab.reference("app", "1")).unwrap();

    // As `docker pull` would leave it: the tag names the new image, the old one has no name.
    lab.push("app", "1", &dockerfile("2", "1000", true));
    lab.release();
    assert_ne!(image_id(&lab.reference("app", "1")), Some(old.clone()));
    lab.check(&stack);

    let observed = lab.u().store().observed();
    let app = &observed.services[&format!("{stack}/app")];
    assert_eq!(app.state, UpdateState::Available, "{:?}", app.reason);
    assert_eq!(app.reason, None);
    assert_eq!(
        app.running.as_ref().map(|r| r.image_id.as_str()),
        Some(old.as_str()),
        "matched by the container's image id"
    );
    assert!(!lab.nothing_to_update(&stack));
}

/// The update flow both store tests run.
fn update_recreates_only_the_changed_service(lab: &mut Lab, suffix: &str) {
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack(suffix, &two_services(lab));
    let compose = lab.compose_bytes(&stack);
    let (old_app, old_side) = (lab.container(&stack, "app"), lab.container(&stack, "side"));
    let old_image = image_id(&lab.reference("app", "1")).unwrap();

    let new_image = lab.push("app", "1", &dockerfile("2", "1000", true));
    // The build retagged the local image; point the tag back, so the runner's tag step shows.
    ok(&["tag", &old_image, &lab.reference("app", "1")]);
    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Available);
    // Research R3 below reads the app's container events. They are followed live through the
    // run: the daemon replays only its last 256 events, and the healthchecks of tests running in
    // parallel push the `create` out of that window before a later `docker events --until` asks.
    let since = chrono::Utc::now().timestamp().to_string();
    let mut follow = Command::new("docker")
        .args([
            "events",
            "--since",
            &since,
            "--filter",
            "type=container",
            "--filter",
            &format!("label=com.docker.compose.project={stack}"),
            "--filter",
            "label=com.docker.compose.service=app",
            "--format",
            "{{.Action}} {{.Actor.ID}}",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("docker events runs");
    let run = lab.update(&stack);
    std::thread::sleep(Duration::from_millis(500));
    let _ = follow.kill();
    let events = stdout(&follow.wait_with_output().unwrap());
    assert_eq!(outcome(&run), Some(StackOutcome::Updated), "{run:?}");

    let new_app = lab.container(&stack, "app");
    assert_ne!(new_app, old_app, "app recreated");
    assert_eq!(lab.container(&stack, "side"), old_side, "side untouched");
    assert_eq!(
        lab.compose_bytes(&stack),
        compose,
        "compose file byte-identical"
    );
    assert_eq!(image_id(&lab.reference("app", "1")), Some(new_image));
    let previous = keep::tag(&stack, "app", KeepKind::Previous);
    assert_eq!(
        image_id(&previous),
        Some(old_image),
        "the old image is kept"
    );

    // Research R3: compose creates the replacement before it stops the old container.
    let lines: Vec<(&str, &str)> = events.lines().filter_map(|l| l.split_once(' ')).collect();
    // `compose ps -q` and the events may print ids of different lengths.
    let same = |a: &str, b: &str| !a.is_empty() && (a.starts_with(b) || b.starts_with(a));
    let created = lines
        .iter()
        .position(|(action, id)| *action == "create" && same(id, &new_app))
        .unwrap_or_else(|| panic!("no create of the new container: {lines:?}"));
    let stopped = lines
        .iter()
        .position(|(action, id)| matches!(*action, "stop" | "kill") && same(id, &old_app))
        .unwrap_or_else(|| panic!("no stop of the old container: {lines:?}"));
    assert!(created < stopped, "created before stopped: {lines:?}");
}

#[test]
fn an_update_recreates_only_the_changed_service_and_leaves_compose_byte_identical() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    update_recreates_only_the_changed_service(&mut lab, "upd");
}

/// SC-005.
#[test]
fn an_image_that_turns_root_is_held_with_tag_and_container_unchanged() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("root", &two_services(&lab));
    let container = lab.container(&stack, "app");
    let image = image_id(&lab.reference("app", "1")).unwrap();

    lab.push("app", "1", &dockerfile("2", "root", true));
    // The push retagged the local image; point the tag back as a plain pull would have left it.
    ok(&["tag", &image, &lab.reference("app", "1")]);
    lab.check(&stack);
    if lab.nothing_to_update(&stack) {
        // Held at check time: nothing runs.
        assert_eq!(lab.state(&stack, "app"), UpdateState::Held);
    } else {
        let run = lab.update(&stack);
        assert_eq!(outcome(&run), Some(StackOutcome::Held), "{run:?}");
        assert!(image_id(&keep::tag(&stack, "app", KeepKind::Held)).is_some());
    }
    assert_eq!(
        lab.container(&stack, "app"),
        container,
        "container unchanged"
    );
    assert_eq!(
        image_id(&lab.reference("app", "1")),
        Some(image),
        "tag unchanged"
    );
}

#[test]
fn the_same_update_on_the_containerd_image_store() {
    if !enabled() {
        return;
    }
    let info = ok(&["info", "--format", "{{json .DriverStatus}}"]);
    if !info.contains("io.containerd.snapshotter.v1") {
        eprintln!("skipped: the daemon does not use the containerd image store ({info})");
        return;
    }
    let mut lab = Lab::new();
    update_recreates_only_the_changed_service(&mut lab, "ctrd");
}

// --- US3 (T063) ----------------------------------------------------------------------------------

/// SC-004.
#[test]
fn a_failing_healthcheck_is_rolled_back_in_time_and_skipped_next_pass() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("rollback", &two_services(&lab));
    let old_image = image_id(&lab.reference("app", "1")).unwrap();

    lab.push("app", "1", &dockerfile("2", "1000", false));
    ok(&["tag", &old_image, &lab.reference("app", "1")]);
    lab.check(&stack);
    let digest = lab.u().store().observed().services[&format!("{stack}/app")]
        .available
        .clone()
        .unwrap()
        .digest;

    let started = Instant::now();
    let run = lab.update(&stack);
    let limit = Duration::from_secs(30 + 60);
    assert!(
        started.elapsed() <= limit,
        "rolled back in {:?}",
        started.elapsed()
    );
    assert_eq!(outcome(&run), Some(StackOutcome::RolledBack), "{run:?}");
    assert_eq!(
        image_id(&lab.reference("app", "1")),
        Some(old_image),
        "the tag points to the old image"
    );
    assert!(image_id(&keep::tag(&stack, "app", KeepKind::Held)).is_some());
    let decisions = lab.u().store().decisions().unwrap();
    assert!(
        decisions
            .rejected
            .iter()
            .any(|r| r.stack == stack && r.service == "app" && r.digest == digest),
        "{:?}",
        decisions.rejected
    );

    lab.check(&stack);
    assert_eq!(lab.state(&stack, "app"), UpdateState::Rejected);
    assert!(lab.nothing_to_update(&stack), "the next pass skips it");
}

// --- US5 (T078) ----------------------------------------------------------------------------------

#[test]
fn cleanup_removes_only_the_image_from_two_updates_ago_and_keeps_one_in_use() {
    if !enabled() {
        return;
    }
    let mut lab = Lab::new();
    lab.settings(true);
    let v1 = lab.push("app", "1", &dockerfile("1", "1000", true));
    lab.push("side", "1", &dockerfile("1", "1000", true));
    let stack = lab.stack("cleanup", &two_services(&lab));

    let v2 = lab.push("app", "1", &dockerfile("2", "1000", true));
    lab.check(&stack);
    assert_eq!(outcome(&lab.update(&stack)), Some(StackOutcome::Updated));
    assert!(image_id(&v1).is_some(), "v1 is the previous image: kept");

    let v3 = lab.push("app", "1", &dockerfile("3", "1000", true));
    lab.check(&stack);
    assert_eq!(outcome(&lab.update(&stack)), Some(StackOutcome::Updated));
    assert!(image_id(&v1).is_none(), "v1, two updates ago, removed");
    assert!(image_id(&v2).is_some(), "v2 is the previous image: kept");

    // Another container uses v2: the next cleanup keeps it.
    let pin = format!("{}-pin", lab.prefix);
    ok(&["run", "-d", "--name", &pin, &v2, "sleep", "86400"]);
    lab.containers.lock().unwrap().push(pin);
    lab.push("app", "1", &dockerfile("4", "1000", true));
    lab.check(&stack);
    assert_eq!(outcome(&lab.update(&stack)), Some(StackOutcome::Updated));
    assert!(image_id(&v2).is_some(), "in use by another container: kept");
    assert!(image_id(&v3).is_some(), "v3 is the previous image: kept");
}

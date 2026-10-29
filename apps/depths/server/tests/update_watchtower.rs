//! Spec 062 quickstart §3 step 6, against a real daemon: a Watchtower beside Depths pauses the
//! Auto queue until an administrator acknowledges it.
//!
//! Opt-in: `DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test update_watchtower`. Without
//! `DEPTHS_IT_DOCKER=1` the test returns at once. Run by the main session only.
//!
//! Its own target, apart from `update_walkthrough`: Watchtower detection reads every container on
//! the daemon, so this test's Watchtower would pause the walkthrough's Auto queues. Run the two one
//! after the other, never at the same time. The Watchtower container is named
//! `depths-it-062wt-<random>-watchtower` and removed with the rest of the [`Lab`] when it drops,
//! also when an assertion fails, so a run that follows finds none. The Watchtower image is removed
//! again only when this test pulled it.

mod support;

use depths_protocol::{AutoPause, StackOutcome, UpdateState};
use serde_json::json;
use support::docker::*;
use support::walk::*;
use support::ws::ask;

const ROOT: &str = "depths-it-062wt";
/// Check now may be asked again after a second.
const ENV: &[(&str, &str)] = &[("DEPTHS_UPDATES_CHECK_GAP", "1s")];
const WATCHTOWER: &str = "containrrr/watchtower:1.7.1";

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

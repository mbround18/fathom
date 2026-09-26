use super::*;
use crate::backend::daemon_client::tests::{Fake, caps_json};
use std::time::Duration;

fn setup(
    dir: &tempfile::TempDir,
) -> (
    BackendState,
    Sender<BackendEventEnvelope>,
    crossbeam_channel::Receiver<BackendEventEnvelope>,
) {
    let mut state = BackendState::new();
    state.daemon.path = Some(dir.path().join("daemon.json"));
    state.cluster.current_context = Some("ctx".into());
    let (tx, rx) = crossbeam_channel::unbounded();
    (state, tx, rx)
}

/// T025: with the preference `off` (the default) no task starts and zero requests are made.
#[tokio::test(start_paused = true)]
async fn preference_off_emits_zero_daemon_requests() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, tx, rx) = setup(&dir);
    let fake = Fake::new(Ok(caps_json()), vec![]);
    // No client is set and mode is off: sync must be a no-op.
    sync_active(&mut state, &tx);
    assert!(!state.daemon.is_running("ctx"));
    // Even a direct start is never reached from the connect hook; the request counter stays 0.
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(fake.requests.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(rx.try_recv().is_err(), "no events when off");
}

#[tokio::test(start_paused = true)]
async fn set_preference_persists_and_off_tears_down() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, tx, rx) = setup(&dir);
    let fake = Fake::new(Ok(caps_json()), vec![]);
    start_with(&mut state, &tx, "ctx", fake.clone(), Timings::default());
    assert!(state.daemon.is_running("ctx"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(fake.requests.load(std::sync::atomic::Ordering::SeqCst) > 0);

    set_preference(&mut state, &tx, "ctx", DaemonMode::Off);
    assert!(!state.daemon.is_running("ctx"));
    let before = fake.requests.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(
        fake.requests.load(std::sync::atomic::Ordering::SeqCst),
        before
    );
    let saw_off = rx.try_iter().any(|e| {
        matches!(
            e.event,
            BackendEvent::System(SystemEvent::DaemonStatusUpdated { ref status, .. })
                if status.state == DaemonState::Off
        )
    });
    assert!(saw_off);

    set_preference(&mut state, &tx, "other", DaemonMode::Auto);
    let f = persistence::load_daemon_from(&dir.path().join("daemon.json"));
    assert_eq!(f.mode_for("other"), DaemonMode::Auto);
    assert_eq!(f.mode_for("ctx"), DaemonMode::Off);
}

#[tokio::test(start_paused = true)]
async fn timeline_request_replays_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, tx, rx) = setup(&dir);
    let fake = Fake::new(
        Ok(caps_json()),
        vec![
            crate::backend::daemon_client::tests::ev_frame(1),
            crate::backend::daemon_client::tests::ev_frame(2),
        ],
    );
    start_with(&mut state, &tx, "ctx", fake, Timings::default());
    tokio::time::sleep(Duration::from_millis(400)).await;
    while rx.try_recv().is_ok() {}
    request_timeline(&state, &tx, "ctx", 1);
    let env = rx.try_recv().unwrap();
    match env.event {
        BackendEvent::System(SystemEvent::DaemonEventsBatch { events, .. }) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].seq, 2);
        }
        other => panic!("{other:?}"),
    }
}

/// US4: the snapshot source follows the active session; off / other contexts get none.
#[tokio::test(start_paused = true)]
async fn snapshot_source_follows_session() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, tx, _rx) = setup(&dir);
    assert!(snapshot_source(&state).is_none());
    assert_eq!(active_mode(&state), DaemonMode::Off);
    let fake = Fake::new(Ok(caps_json()), vec![]);
    start_with(&mut state, &tx, "ctx", fake, Timings::default());
    assert_eq!(
        snapshot_source(&state).map(|s| s.context),
        Some("ctx".into())
    );
    state.cluster.current_context = Some("other".into());
    assert!(
        snapshot_source(&state).is_none(),
        "never used for another context"
    );
    state.cluster.current_context = Some("ctx".into());
    set_preference(&mut state, &tx, "ctx", DaemonMode::Off);
    assert!(snapshot_source(&state).is_none());
    set_preference(&mut state, &tx, "ctx", DaemonMode::Auto);
    assert_eq!(active_mode(&state), DaemonMode::Auto);
}

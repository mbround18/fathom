//! Shared harness for headless GUI tests using `egui_kittest` (AccessKit-based;
//! no window, GPU, or display server required - unlike a real screenshot, this
//! runs anywhere `cargo test` runs).
//!
//! Each test drives a real `FathomApp` with no backend thread attached:
//! `BackendCmd`s it sends just queue up unread, and `BackendEvent`s are
//! injected directly through the same channel a live backend would use
//! (`TestApp::push_event`), letting tests exercise populated states - not
//! just the empty ones a disconnected app starts in.
//!
//! Notes for anyone adding a new file that uses this module:
//! - `egui::ComboBox` exposes its selected text as the node's *value*, not
//!   its *label* - query with `By::new().role(Role::ComboBox).value(...)`,
//!   not `get_by_label`.
//! - Every `Table` row/header checkbox renders the same "☐"/"☑"/"☒" glyph as
//!   its label - `get_all_by_label` + tree-order indexing (header is index 0)
//!   is the way to disambiguate before anything is selected.
//! - When two nodes can share a label (e.g. a details-panel button and a
//!   modal button both titled "Delete"), `get_all_by_label(...).last()` picks
//!   the one rendered latest in the frame - reliable here because
//!   `render_modal` runs after `render_right_panel`/`render_center` in
//!   `FathomApp::update`.
//! - After any click or `push_event`, call `harness.run_steps(2)` before
//!   asserting - one frame to process the input/event, one to reflect the
//!   resulting state.
//! - `Node::click()` is coordinate-based (it caches the node's on-screen rect
//!   at query time, then sends a pointer event at that position on the next
//!   `run_steps`). If a click earlier in the same test changes another
//!   panel's content shape (e.g. switching a details-panel tab), a
//!   *different* widget's on-screen position can shift by the time the
//!   click is actually delivered, landing off-target - this looks exactly
//!   like "the click silently did nothing" and is easy to mistake for an
//!   app bug (traced one all the way to a `BackendCmd` never firing before
//!   finding this). `Node::click_accesskit()` sends a semantic AccessKit
//!   click action targeting the node directly instead of a coordinate, and
//!   isn't affected - reach for it if a `click()` that should obviously work
//!   mysteriously doesn't, especially right after another panel re-rendered.

#![allow(dead_code)] // not every test file uses every helper here

use crossbeam_channel::{Receiver, Sender, unbounded};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use fathom::app::FathomApp;
use fathom::protocol::{BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route};
use std::sync::Mutex;

/// `NavigationView` persists expanded nav groups to `~/.fathom/config.json`, so
/// every test needs an isolated `$HOME` - and since env vars are process-global,
/// tests sharing a process must not mutate `$HOME` concurrently. Serialize with
/// this lock (shared across every test file that uses this module, since Rust
/// statics are per-binary and each integration test file is its own binary -
/// safe either way, this just also covers multiple `#[test]` fns in one file).
static HOME_LOCK: Mutex<()> = Mutex::new(());

pub struct TestApp {
    pub harness: Harness<'static, FathomApp>,
    pub events_tx: Sender<BackendEventEnvelope>,
    pub cmds_rx: Receiver<BackendCmd>,
    #[allow(dead_code)]
    tmp_home: tempfile::TempDir,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl TestApp {
    /// Send a synthetic backend event straight to the GUI, as if a live backend
    /// had replied, and let the app process it.
    pub fn push_event(&mut self, route: Route, event: BackendEvent) {
        self.events_tx
            .send(BackendEventEnvelope::new(
                1,
                route,
                Correlation::None,
                event,
            ))
            .expect("gui channel open");
        self.harness.run_steps(2);
    }

    /// Expand a top-level nav group (no-op if already expanded) then click a
    /// link inside it. `item_label` must match the exact rendered text
    /// (NavigationView prefixes items with leading spaces for indentation).
    pub fn navigate(&mut self, group: &str, item_label: &str) {
        let group_button = format!("▶ {group}");
        if self.harness.query_by_label(&group_button).is_some() {
            self.harness.get_by_label(&group_button).click();
            self.harness.run_steps(2);
        }
        self.harness.get_by_label(item_label).click();
        self.harness.run_steps(2);
    }

    /// Drain and discard every `BackendCmd` sent so far, so later assertions
    /// only see commands dispatched by the test's own actions.
    pub fn drain_commands(&self) {
        while self.cmds_rx.try_recv().is_ok() {}
    }

    /// `drain_commands()`, then return the single next `BackendCmd` sent -
    /// panics if none arrives or more than one does, so tests stay precise
    /// about exactly what a click dispatched.
    pub fn next_command(&self) -> BackendCmd {
        let cmd = self
            .cmds_rx
            .try_recv()
            .expect("expected exactly one BackendCmd to have been sent");
        assert!(
            self.cmds_rx.try_recv().is_err(),
            "expected exactly one BackendCmd, but more than one was sent"
        );
        cmd
    }
}

pub fn setup() -> TestApp {
    let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp_home = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("HOME", tmp_home.path());
    }

    let (events_tx, gui_rx) = unbounded();
    let (backend_tx, cmds_rx) = unbounded();

    let mut harness = Harness::new_eframe(move |_cc| FathomApp::new(gui_rx, backend_tx));
    harness.run_steps(2);

    let app = TestApp {
        harness,
        events_tx,
        cmds_rx,
        tmp_home,
        _guard: guard,
    };
    // Startup pushes several commands (RefreshContexts, Initialize, the
    // default Overview section's on_mount fetches, ...) - not what any test
    // here cares about.
    app.drain_commands();
    app
}

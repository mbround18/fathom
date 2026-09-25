//! T013 (031-hosted-server-mode): two backends in one process must not leak command errors or
//! outcomes into each other (`events::take_cmd_errors` is task-local; there is no other
//! process-global backend state that carries per-command results).

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use fathom::backend::access::KubeconfigAccess;
use fathom::backend::audit_log::LocalFileSink;
use fathom::bridge::spawn_backend_with;
use fathom::config::Args;
use fathom::protocol::{BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest, RequestId};

fn args() -> Args {
    Args {
        mcp_stdio: false,
        only_mcp: false,
        disable_mcp: true,
    }
}

fn finished(rx: &Receiver<BackendEventEnvelope>, id: RequestId) -> (bool, Option<String>) {
    loop {
        let env = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("CommandFinished for request");
        if let BackendEvent::CommandFinished { ok, error } = env.event
            && matches!(env.correlation, fathom::protocol::Correlation::Request(r) if r == id)
        {
            return (ok, error);
        }
    }
}

#[test]
fn concurrent_backends_do_not_share_command_errors() {
    let mut chans = Vec::new();
    for _ in 0..2 {
        let (cmd_tx, cmd_rx) = unbounded();
        let (ev_tx, ev_rx) = unbounded();
        let handle = spawn_backend_with(
            cmd_rx,
            ev_tx,
            args(),
            Arc::new(KubeconfigAccess),
            Arc::new(LocalFileSink),
            None,
            None,
        );
        chans.push((cmd_tx, ev_rx, handle));
    }
    // Backend 0: a command that fails with no cluster connection. Backend 1: one that succeeds.
    // Fire both without waiting so the two run concurrently.
    let bad = RequestId(1);
    let good = RequestId(1);
    chans[0]
        .0
        .send(CmdRequest::new(bad, BackendCmd::RefreshPods))
        .unwrap();
    chans[1]
        .0
        .send(CmdRequest::new(good, BackendCmd::SimulateSlowCall))
        .unwrap();

    let (ok0, err0) = finished(&chans[0].1, bad);
    let (ok1, err1) = finished(&chans[1].1, good);
    assert!(!ok0, "backend 0 must report its own failure");
    assert!(err0.is_some());
    assert!(
        ok1,
        "backend 1 must not inherit backend 0's error: {err1:?}"
    );
    assert!(err1.is_none());

    for (tx, _rx, handle) in chans {
        tx.send(CmdRequest::new(RequestId(2), BackendCmd::Shutdown))
            .unwrap();
        drop(tx);
        handle.join().ok();
    }
}

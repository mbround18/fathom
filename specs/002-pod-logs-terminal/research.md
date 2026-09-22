# Phase 0 Research: Pod Logs & Terminal

## R1: kube-rs 2.0.1 exec/attach API

**Decision**: Use `Api<Pod>::exec(name, command, &AttachParams)` (from `kube::api`, gated by the
already-enabled `"ws"` feature — confirmed present in the workspace `Cargo.toml`, no new Rust
dependency needed). Build params with `AttachParams::interactive_tty()` (source-verified default:
`stdin: true, stdout: true, stderr: false, tty: true` — stderr is intentionally folded into stdout
when `tty: true`, matching `kubectl exec -it` behavior and avoiding a second reader to relay), then
`.container(name)` when the pod has more than one container. `command` uses a two-attempt fallback (user decision, clarified 2026-09-21): first try
`vec!["/bin/bash"]`; if that `Api::exec` call errors (container has no `/bin/bash`), retry once
with `vec!["/bin/sh"]`. If the second attempt also errors, that error IS the "no usable shell" case
FR-013 requires surfacing — no third attempt. Bash first because it's a materially nicer
interactive shell (history, arrow-key line editing, tab completion) when present, and the fallback
cost is only paid once, on session start, not per-keystroke.

`Api::exec` returns `AttachedProcess` (source: `kube-client-2.0.1/src/api/remote_command.rs`).
Source-verified surface:
- `.stdin() -> Option<impl AsyncWrite + Unpin>` — take-once.
- `.stdout() -> Option<impl AsyncRead + Unpin>` — take-once (carries stderr too, since `interactive_tty()` sets `stderr: false`).
- `.terminal_size() -> Option<TerminalSizeSender>` where `TerminalSizeSender = futures::channel::mpsc::Sender<TerminalSize>` — only present because `tty: true` was set. Sending requires `futures::SinkExt::send(...).await`, not tokio's `mpsc::Sender`.
- `.take_status() -> Option<impl Future<Output = Option<Status>>>` — resolves when the remote process exits; used to detect session end distinct from an abort.
- `.abort()` — aborts the internal background task immediately.
- `TerminalSize { pub width: u16, pub height: u16 }` (PascalCase serde).

**Rationale**: `AttachedProcess::new()` already spawns its own `tokio::spawn`ed internal
`start_message_loop` task that owns the actual websocket connection and multiplexes
stdin/stdout/resize/status over channel-byte-tagged frames — this is the crate's job, not ours.
Fathom's `exec.rs` only needs to (a) call `Api::exec`, (b) extract `stdin`/`stdout`/`terminal_size`
handles immediately, and (c) keep the `AttachedProcess` value itself alive for the session's
lifetime so `.abort()` can be called on stop — dropping it early would still let the internal task
run to completion undriveable (no way to abort it later), which would leak the session.

**Alternatives considered**: `Api::attach` (attaches to an already-running process 1, not a new
exec) — rejected, spec requires an interactive shell, not attaching to the container's PID 1 which
may not be a shell and may terminate the pod if killed. Building the SPDY/exec protocol by hand —
rejected, `kube`'s `"ws"` feature already implements the full k8s remotecommand v4/v5 protocol
correctly (channel bytes, stream-close signaling); reimplementing it would violate Principle II
(one contract, not reinventing cluster protocol handling) for no benefit.

## R2: Session lifecycle — mirroring `LogStreamState`

**Decision**: New `ExecSessionState` in `src/backend/state.rs`, structurally mirroring the existing
`LogStreamState { log_task: Option<JoinHandle<()>>, next_stream_id: u64 }`:

```rust
pub struct ExecSessionState {
    attached: Option<AttachedProcess>,   // held alive for .abort() on stop
    relay_task: Option<JoinHandle<()>>,  // owns the stdout-read loop, per below
    stdin_writer: Option<Box<dyn AsyncWrite + Unpin + Send>>,
    resize_tx: Option<TerminalSizeSender>,
}
impl ExecSessionState {
    fn stop(&mut self) { if let Some(mut p) = self.attached.take() { p.abort(); } if let Some(t) = self.relay_task.take() { t.abort(); } self.stdin_writer = None; self.resize_tx = None; }
}
```

One active session application-wide (spec FR-012): `StartPodExec` always calls `.stop()` on any
existing session first, exactly like `StartPodLogs`'s existing "stop-before-start" convention.

**Stdin/resize path — no relay task needed for those**: since `SendPodExecInput` and
`ResizePodExec` command handlers are already `async fn`s invoked one-at-a-time from the backend's
command loop, they can write directly to the stored `stdin_writer` (`AsyncWriteExt::write_all`)
and send directly to the stored `resize_tx` (`SinkExt::send`) — no extra channel/relay-task
indirection needed for the inbound direction. Only the *outbound* (stdout → frontend event) side
needs a background task, since output arrives continuously and must be pushed as events rather
than pulled on command.

**Output relay task**: on `StartPodExec`, after extracting `stdout`, `tokio::spawn` a loop reading
chunks via `tokio::io::AsyncReadExt::read` and emitting `StreamEvent::PodExecOutputChunk{stream_id,
data}` per read (base64 or raw bytes — see R4), ending with `StreamEvent::StreamClosed{stream_id,
reason}` when the reader hits EOF or errors — reusing the existing `StreamClosed` event exactly as
Logs does, rather than inventing a parallel event.

**Shared stream ID space**: move `next_stream_id` off `LogStreamState` onto `BackendState` itself
(`fn next_stream_id(&mut self) -> StreamId`), so Log and Exec streams draw from one counter and
`stream_id` stays a reliable unique key for both kinds of session in `store.streams` on the
frontend — avoids the two features silently colliding on ID `1` in the frontend reducer.

**Rationale**: This is a direct, minimal-diff application of the existing singleton pattern already
proven correct for Logs (Principle II: reuse the established contract style; constitution's "Fast
and Quiet" — bounded, single active resource, no orphaned tasks).

**Alternatives considered**: A generic multi-session registry (`HashMap<SessionId, ...>`) —
rejected as unnecessary complexity; spec explicitly scopes to one active Terminal session
app-wide (Assumptions section), and the existing codebase convention for exactly this shape
(one active stream) is the singleton, not a map.

## R3: New protocol variants

**Decision**:
- `BackendCmdV1`: `StartPodExec{namespace, name, container: Option<String>}`,
  `SendPodExecInput{data: String}` (base64-encoded, see R4), `ResizePodExec{cols: u16, rows: u16}`,
  `StopPodExec`. No `stream_id` param on `SendPodExecInput`/`ResizePodExec`/`StopPodExec` — like
  `StopPodLogs`, they target "the current active session" implicitly (singleton), keeping the
  command surface symmetric with the existing log commands.
- `StreamEvent`: `PodExecReady{stream_id}` (emitted once attach succeeds — lets the frontend flip
  from "attaching" to "active", satisfying FR-009/FR-013's requirement for a distinguishable
  attach-failed vs. attach-succeeded state), `PodExecOutputChunk{stream_id, data: String}` (base64),
  reuse `StreamClosed{stream_id, reason}` for both normal exit and error termination (reason string
  distinguishes them, same convention `StreamClosed` already uses for log-stream end reasons).
- `mutation.rs` classification: all four new commands are explicitly reviewed and classified as
  **non-mutating** to the Kubernetes resource graph (they don't touch object specs/state — same
  trust tier as `StartPodLogs`), matching Constitution Principle I's analysis in plan.md. This
  classification only governs Fathom's own optimistic-UI/confirmation gating — it is a separate
  concern from the promo pipeline's `WRITE_CMD` regex (R6), which stays untouched.

**Rationale**: Symmetry with the existing `StartPodLogs`/`StopPodLogs` pair keeps the frontend
wiring and backend `cmd/workloads.rs` dispatch pattern familiar and reviewable at a glance.

**Alternatives considered**: Including `stream_id` explicitly on every follow-up command
(`SendPodExecInput{stream_id, data}`) — rejected as redundant given the singleton constraint; adds
a class of "wrong/stale stream_id" error the singleton design makes structurally impossible to hit
in the first place.

## R4: Binary data over the JSON protocol

**Decision**: `PodExecOutputChunk.data` and `SendPodExecInput.data` are base64-encoded `String`
fields. The existing protocol (`serde` + ts-rs-generated TS types) is JSON-based; terminal
I/O is technically raw bytes (escape sequences, non-UTF8-safe control bytes from some programs),
so base64 is the safe, simple choice over assuming valid UTF-8. The `base64` crate is already a
workspace dependency (used for encryption/integration features per `Cargo.toml`).

**Rationale**: Avoids a protocol-wide binary-frame mechanism (WebSocket-style) that doesn't exist
today in Fathom's Tauri IPC layer — reuses the existing JSON command/event channel exactly as-is
(Principle II: one contract).

**Alternatives considered**: Assume-UTF8 and pass raw strings — rejected, real shells routinely
emit non-UTF8-safe byte sequences (raw terminal control codes) that would panic or corrupt a
naive `String::from_utf8` conversion.

## R5: Frontend terminal rendering

**Decision**: `xterm` + `@xterm/addon-fit` as new `ui/` dependencies. `xterm.js` is the de facto
standard for browser/Electron/Tauri terminal rendering (VS Code, Hyper, countless others use it),
handles ANSI escape sequences, virtualized/bounded scrollback natively (satisfies constitution
Principle V without extra bookkeeping), and its `FitAddon` gives a one-line way to compute
terminal cols/rows from the pane's pixel size for the resize-propagation requirement (FR-010).

**Rationale**: Building a terminal emulator (ANSI parsing, cursor handling, scrollback) from
scratch would be a large, error-prone undertaking wildly out of proportion to this feature's scope.

**Alternatives considered**: `hterm`, raw `<pre>` + naive ANSI stripping — rejected; neither
handles interactive full-screen programs (`top`, `vi` — explicitly named in spec Acceptance
Scenario 2.3) correctly.

**Scrollback bound** (user decision, clarified 2026-09-21): `scrollback: 1000` in the
`xterm.Terminal` constructor options — generous for interactive debugging without holding
unbounded memory (constitution Principle V). Chosen independently of the Logs tab's `RingBuffer`
size since the two are different mechanisms (xterm.js's own internal buffer vs. a plain string
ring buffer) with no shared-consistency requirement between them.

## R6: Promo pipeline read-only guarantee

**Decision**: The promo capture pipeline (`ui/tests/promo/capture.spec.ts`,
`ui/tests/promo/helpers.ts`) is left completely untouched with respect to exec: the new
`StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` commands are simply never emitted
by any promo scene. The existing `WRITE_CMD` regex in `helpers.ts` continues to guard
delete/scale/restart-style mutations exactly as before — it is not extended or relied upon for
exec safety, because exec commands are categorically different (they don't match the regex's
mutation vocabulary at all, and relying on a regex to keep a live shell out of a demo cluster
would be a fragile safety mechanism for something this consequential).

Instead, the Terminal promo page uses a **staged screenshot asset**: a PNG captured once, by hand,
of the real Terminal tab running against a disposable/sandboxed pod (not the shared live demo
cluster used for automated capture), checked into `ui/tests/promo/assets/` (new directory,
mirroring where other static promo assets already live) and composed into the brochure by
`compose.spec.ts` like any other image asset — no live automation opens the Terminal tab, ever.

**Rationale**: Directly satisfies spec FR-015 / User Story 3's explicit hard constraint, and
matches the already-decided architecture in `plan.md`'s Summary and Constitution Check.

**Alternatives considered**: Extending `WRITE_CMD` to also block exec commands during promo runs —
rejected as insufficient on its own; the promo pipeline should never construct/send these commands
in the first place, not merely have them filtered after the fact. Using a mocked/fake terminal
pane just for the screenshot — rejected, spec's Success Criterion SC-004 requires the brochure
show real, non-mocked evidence of the capability.


**Screenshot pod source** (user decision, clarified 2026-09-21): the existing `web` Deployment in
`dev/sandbox/workloads.yaml` (`nginxinc/nginx-unprivileged:stable-alpine`, which has `/bin/sh`) —
no new manifest needed. It runs in the local dev sandbox cluster, not the shared live demo cluster
the automated promo pipeline targets, so it satisfies the "never against the shared live demo
cluster" requirement by construction.

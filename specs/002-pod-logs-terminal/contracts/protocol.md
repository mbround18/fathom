# Protocol Contract: Pod Logs & Terminal

Fathom's external interface (per Constitution Principle II) is its internal versioned
frontend/backend protocol (`src/protocol/v1/`, ts-rs-generated into
`ui/src/protocol/generated/`). This feature's additions:

## Commands (`src/protocol/v1/cmd.rs`, `BackendCmdV1`)

Logs (existing, unchanged — documented here for completeness only):
```rust
StartPodLogs { namespace: String, name: String, container: Option<String> }
StopPodLogs
```

Terminal (new):
```rust
StartPodExec { namespace: String, name: String, container: Option<String> }
SendPodExecInput { data: String }   // base64-encoded bytes
ResizePodExec { cols: u16, rows: u16 }
StopPodExec
```

Contract rules:
- `StartPodExec` MUST stop any existing exec session first (singleton, FR-012), mirroring
  `StartPodLogs`'s relationship to `StopPodLogs` — except Logs has no cross-session singleton
  constraint and Terminal does.
- `SendPodExecInput`/`ResizePodExec`/`StopPodExec` implicitly target the one active session; if
  none is active, they are no-ops (mirrors `StopPodLogs` being a no-op when nothing is streaming).
- None of these four commands are ever sent by the automated promo capture pipeline
  (`ui/tests/promo/`) — enforced by the capture scenes simply never constructing them, not by the
  `WRITE_CMD` regex in `ui/tests/promo/helpers.ts` (research.md R6). That regex is NOT extended for
  this feature.

## Events (`src/protocol/v1/event.rs`, `StreamEvent`)

Logs (existing, unchanged):
```rust
PodLogsChunk { stream_id: StreamId, lines: Vec<String> }
StreamClosed { stream_id: StreamId, reason: String }
```

Terminal (new):
```rust
PodExecReady { stream_id: StreamId }
PodExecOutputChunk { stream_id: StreamId, data: String }   // base64-encoded bytes
```
Reuses `StreamClosed { stream_id, reason }` for both normal exit and error/attach-failure
termination — `reason` distinguishes them (e.g. `"exit"` vs. an error message surfaced verbatim
for FR-013's permission-denied / no-shell cases).

## Frontend consumption contract

- `ui/src/store/handlers/stream.ts` gains a case for `PodExecReady`/`PodExecOutputChunk` alongside
  the existing `PodLogsChunk`/`StreamClosed` cases, updating `store.streams[stream_id]` (see
  data-model.md's `ExecStreamState`).
- `Inspector.tsx` gates the "Logs" tab to Pod resources (any status) and the "Terminal" tab to Pod
  resources in a running state (FR-001, FR-007) — both send their `Start*` command on tab-open and
  `Stop*` on tab-close/Inspector-close/resource-switch (FR-005, FR-011).
- `PodTerminalTab.tsx` wires xterm.js's `onData` (keystrokes) to `SendPodExecInput` and its resize
  observer (via `FitAddon`) to `ResizePodExec`.

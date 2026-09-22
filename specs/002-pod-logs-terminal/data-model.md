# Phase 1 Data Model: Pod Logs & Terminal

## Log Stream

Backend-owned; already fully implemented (`src/backend/state.rs::LogStreamState`,
`src/backend/pods.rs::start_logs`). No changes to backend state shape — only a UI consumer is new.

| Field | Type | Notes |
|---|---|---|
| `stream_id` | `StreamId` (u64) | Assigned from the (now-shared, see R2) `BackendState::next_stream_id()` counter |
| `namespace` / `name` / `container` | `String` / `Option<String>` | Identifies the log source; `container` required when pod has >1 container (FR-003) |
| lifecycle | `streaming \| ended \| error` | Frontend-tracked via `StreamEvent::PodLogsChunk` (implies streaming) vs. `StreamClosed` (ended/error, `reason` distinguishes) |

Frontend representation: `store.streams[stream_id]` (`ui/src/store/handlers/stream.ts`, existing
`LogStreamState` shape: `RingBuffer<string>` bounded history, `kind: 'logs'`, `rev`, `follow`
(bool — auto-scroll-to-bottom flag, toggled by scroll position per FR-004), `status`.

## Terminal Session

New. One active instance application-wide (spec Key Entities, FR-012).

**Backend** (`src/backend/state.rs::ExecSessionState`, see research.md R2):

| Field | Type | Notes |
|---|---|---|
| `stream_id` | `StreamId` | Same shared counter as Log Stream |
| `attached` | `Option<AttachedProcess>` | Held for lifetime of session; `.abort()` on stop |
| `relay_task` | `Option<JoinHandle<()>>` | Owns the stdout→event relay loop |
| `stdin_writer` | `Option<Box<dyn AsyncWrite + Unpin + Send>>` | Written to directly by `SendPodExecInput` handler |
| `resize_tx` | `Option<TerminalSizeSender>` | Sent to directly by `ResizePodExec` handler |

State transitions: `(none) → attaching → active → ended | error`. `attaching` is a **frontend-only**
state (the moment between sending `StartPodExec` and receiving `PodExecReady` or a `StreamClosed`
error) — the backend has no explicit "attaching" state of its own; it either successfully builds
an `ExecSessionState` (emit `PodExecReady`) or the `Api::exec` call errors (emit `StreamClosed`
with an error reason, no `ExecSessionState` created). Starting a new session while one exists
always calls `.stop()` on the old one first (singleton, FR-012).

**Frontend**: extends `store.streams` with a `kind: 'exec'` variant alongside the existing
`kind: 'logs'`:

```ts
type ExecStreamState = {
  kind: 'exec';
  status: 'attaching' | 'active' | 'ended' | 'error';
  reason?: string;       // populated on ended/error, from StreamClosed.reason
  container?: string;    // chosen container, for display
};
```

No ring buffer on the frontend for exec output — xterm.js itself owns and renders the
scrollback (R5); the store only tracks session lifecycle/status, not the byte stream content.

## Relationships

- Both entities are scoped to exactly one Pod (`namespace` + `name`), selected via the Inspector's
  currently-inspected resource — no new selection/identity concept beyond what Inspector.tsx
  already tracks.
- Both entities' lifecycle is tied 1:1 to their Inspector tab being open (FR-005, FR-011): tab
  close/switch/Inspector-close triggers the corresponding `Stop*` command.
- A Log Stream and a Terminal Session may be simultaneously active for different pods (Log Stream
  has no cross-pod singleton constraint — only Terminal does, per FR-012's application-wide limit)
  or even the same pod, without interaction — they are independent `stream_id`s sharing only the
  counter, not any other state.

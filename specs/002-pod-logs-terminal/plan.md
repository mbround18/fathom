# Implementation Plan: Pod Logs & Terminal

**Branch**: `002-pod-logs-terminal` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-pod-logs-terminal/spec.md`

## Summary

Give the Inspector two new Pod-only tabs — Logs and Terminal — and use both to enrich the promo
brochure. Logs reuses backend plumbing that already exists end-to-end (`pods::start_logs`,
`BackendCmdV1::StartPodLogs`/`StopPodLogs`, `StreamEvent::PodLogsChunk`/`StreamClosed`,
`store.streams` ring buffer) and only needs a UI consumer. Terminal is new end to end: a
`kube::Api::exec` (tty) attach on the backend, a bidirectional stdin/stdout relay over the
existing command/event protocol, and an xterm.js-rendered pane on the frontend. Both follow the
codebase's existing singleton-session convention (`LogStreamState`: stop-before-start, one active
session) rather than introducing multi-session bookkeeping. The promo pipeline's read-only
guarantee is preserved exactly as-is: Logs is captured live against the demo cluster (it's a
read-only stream, like the existing Flux/Hubble scenes); Terminal is captured once, by hand, as a
static screenshot asset — the automated pipeline never opens or drives the Terminal tab.

## Technical Context

**Language/Version**: Rust (edition 2024, backend) / TypeScript strict (frontend)

**Primary Dependencies**: `kube` 2.0.1 (`ws` feature, already enabled — provides `Api::exec`),
`tokio`, `crossbeam-channel` (backend, all already workspace deps); `xterm` + `@xterm/addon-fit`
(new frontend deps); React, Zustand-style store (`ui/src/store`), shadcn/ui `Tabs` (existing
frontend deps)

**Storage**: N/A — both features are live, in-memory session state only; nothing persisted to
`~/.fathom/`

**Testing**: `cargo test` (backend unit tests for the exec/log aggregation helpers, following
`flux.rs`/`hubble.rs` conventions), Playwright E2E against the scripted fake backend
(`ui/tests/live` / `ui/tests/e2e` conventions) driving the UI by injecting `StreamEvent`s and
asserting emitted `BackendCmdV1`s, Vitest unit tests for store reducers (`stream.ts`)

**Target Platform**: Tauri v2 desktop app — Ubuntu 26.04, Windows 11, macOS 26 (Tier 1, per
constitution)

**Project Type**: Desktop app (Rust backend + Tauri/React frontend), single repo

**Performance Goals**: Per constitution Principle V — input handled under 100ms even mid-stream;
log/terminal output must be virtualized/bounded, not held as unbounded history

**Constraints**: Exactly one active Terminal session application-wide (spec FR-012); promo
capture pipeline must never send exec-related commands during an unattended run (spec FR-015);
no change to the existing write-action confirmation model

**Scale/Scope**: Two new Inspector tabs, ~4 new protocol command variants, ~3 new protocol event
variants, one new backend module (`exec.rs`), two new frontend panel components, one new frontend
dependency, promo capture/compose updates

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation** — PASS. Logs and Terminal are not cluster-mutating
  actions (no delete/scale/restart-style confirmation needed); they read/attach only. Terminal
  *can* be used to run mutating commands inside a container, but that's the operator's own shell
  session, not a Fathom-initiated mutation — same trust boundary as `kubectl exec` today. No
  optimistic UI: the tab shows "attaching" until the backend confirms the stream/session is live.
- **II. One Contract, Backend Owns the Cluster** — PASS. All new capability (log start/stop, exec
  attach/input/resize/stop) is added to `src/protocol/v1/{cmd,event}.rs` and implemented only in
  the Rust backend (`src/backend/exec.rs`, extending `src/backend/pods.rs`'s existing log code).
  The frontend only sends commands and renders events via generated ts-rs bindings — no direct
  cluster access, no new `~/.fathom/` reads/writes.
- **III. Tested at the Boundaries** — PASS (planned). New protocol variants get serialization
  round-trip tests; the exec output-aggregation and log-tab container-selection logic get unit
  tests; new Playwright E2E scenarios drive both tabs against the fake backend fixture.
- **IV. Density Scales With Width** — PASS. Both tabs live inside the existing Inspector tab
  strip (Details/YAML/Events pattern) — no new layout slot, no new width-class behavior needed;
  they inherit the Inspector's existing responsive behavior.
- **V. Fast and Quiet** — PASS (design requirement carried into Phase 1). Log lines use the
  existing `RingBuffer` (bounded); terminal output is rendered by xterm.js, which is itself
  virtualized/bounded by design (fixed scrollback). Streams must stop the instant their tab
  closes (spec FR-005/FR-011) so idle CPU stays near zero, matching the existing log-stream
  stop-on-leave pattern already implemented for `BackendCmd::StopPodLogs`.
- **VI. User Data Is Sacred** — PASS. Nothing here touches `~/.fathom/`; no secrets are
  persisted. Terminal session input/output is never logged to disk.

No violations. Complexity Tracking table is not needed.

**Post-Phase-1 re-check** (after research.md/data-model.md/contracts/quickstart.md were written):
still PASS on all six principles. Nothing in the detailed design introduces a violation — the
base64-over-JSON encoding (research.md R4) and shared `next_stream_id()` counter (R2) are internal
implementation details, not protocol/trust-boundary changes; the staged-screenshot approach for
Terminal (R6) is what keeps Principle I/VI intact for promo capture specifically.

## Project Structure

### Documentation (this feature)

```text
specs/002-pod-logs-terminal/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/            # Phase 1 output
└── tasks.md              # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── cmd.rs            # + StartPodExec, SendPodExecInput, ResizePodExec, StopPodExec
│   ├── event.rs          # + PodExecOutputChunk, PodExecReady, PodExecExitCode; reuse StreamClosed
│   └── mutation.rs        # classify new exec commands (non-mutating, but reviewed against gate I)
├── backend/
│   ├── state.rs            # + ExecSessionState (mirrors LogStreamState), shared next_stream_id()
│   ├── pods.rs              # next_stream_id() call site updated (shared counter refactor);
│   │                          start_logs itself unchanged
│   ├── exec.rs              # NEW: start_exec / send_input / resize / stop, using kube Api::exec
│   └── cmd/workloads.rs      # + match arms wiring the 4 new BackendCmdV1 variants to exec.rs

ui/
├── src/
│   ├── protocol/generated/    # ts-rs regenerated: new cmd/event TS types
│   ├── store/
│   │   ├── index.ts              # + ExecSessionState in StoreData
│   │   └── handlers/stream.ts    # + exec output/ready/exit handling alongside PodLogsChunk
│   └── components/
│       ├── Inspector.tsx         # + Logs, Terminal TabsTrigger/TabsContent (Pod-only, gated)
│       └── panels/
│           ├── PodLogsTab.tsx    # NEW: container picker + follow-to-bottom log view
│           └── PodTerminalTab.tsx # NEW: container picker + xterm.js pane, resize wiring
├── package.json                  # + xterm, @xterm/addon-fit
└── tests/
    ├── promo/
    │   ├── capture.spec.ts        # + wide-logs scene (live); Terminal via staged asset only
    │   ├── compose.spec.ts        # + logs shot + terminal (staged) page in SHOT_META/pages
    │   └── helpers.ts              # WRITE_CMD regex UNCHANGED; exec cmds simply never sent by capture
    └── live|e2e/                   # + Playwright scenarios for Logs/Terminal against fake backend
```

**Structure Decision**: Single-repo desktop app, existing `src/` (Rust backend) + `ui/`
(Tauri/React frontend) split. This feature adds one new backend module (`exec.rs`) beside the
existing `pods.rs`, and two new frontend panel components beside the existing `panels/`
directory — no new top-level directories.

## Complexity Tracking

*No violations — table not needed.*

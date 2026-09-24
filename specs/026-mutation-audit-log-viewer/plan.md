# Implementation Plan: Local Mutation Audit-Log Viewer

**Branch**: `026-mutation-audit-log-viewer` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/026-mutation-audit-log-viewer/spec.md`

## Summary

Persist an append-only, bounded local record of every mutating action Fathom performs (action
type, cluster context, target resource, timestamp, outcome), by hooking a new
`audit_summary()` classification and a persistence write into the existing terminal
`BackendEvent::CommandFinished` emission point in `run_cmd` (`src/backend.rs`) — no new
mutation-tracking mechanism. Entries are stored in a new flat JSON file,
`~/.fathom/audit-log.json`, using the same `load_json_file`/`save_json_file` convention already
used for `ports.json`/`cluster-settings.json`/`config.json` (no new storage technology; no
SQLite/sled exists or is needed in this codebase). A new read-only protocol command
(`RefreshAuditLog`) exposes the bounded (500-entry) history to a new read-only UI pane that
filters/searches entirely client-side.

## Technical Context

**Language/Version**: Rust (edition 2024, per constitution) for the backend; TypeScript (strict)
for the frontend.

**Primary Dependencies**: Existing workspace deps only — `serde`/`serde_json` (already used for
all `~/.fathom/*.json` persistence), `uuid` (check if already a dependency; if not, a UUID can be
generated via the existing `RequestId`/`ids.rs` machinery or a minimal additive dependency —
resolved during implementation, no new heavyweight dependency needed). Frontend: existing
Zustand store, shadcn/ui components, Playwright (already present).

**Storage**: Flat JSON file, `~/.fathom/audit-log.json` (new file), read/written via the existing
`src/backend/persistence.rs` `load_json_file`/`save_json_file` helpers. See `research.md` R2 for
why this reuses the existing mechanism rather than introducing SQLite/sled/etc.

**Testing**: `cargo test` (Rust unit tests for `audit_summary()` classification and the
load/save/retention-truncation round trip), Vitest (frontend unit tests for filter/search logic),
Playwright (`ui/tests/e2e/audit-log.spec.ts`, new — drives the UI against the scripted fake
backend per Constitution Principle III).

**Target Platform**: Same as the rest of Fathom — Tauri v2 desktop app, Tier 1: Ubuntu 26.04,
Windows 11, macOS 26 (Constitution "Supported Platforms").

**Project Type**: Existing single Tauri desktop app (Rust backend + `ui/` React/TS frontend) —
not a new project type; this feature is purely additive within the existing structure.

**Performance Goals**: Audit-write adds <100ms to command completion (SC-003); viewer filter/
search over up to ~500 in-memory entries responds within the existing <100ms input-handling bar
(Constitution Principle V) — well within budget since 500 entries is trivial to filter in memory.

**Constraints**: Audit persistence MUST be best-effort and MUST NOT block, delay, or alter the
outcome of the mutating action itself (FR-011). MUST NOT read/merge cluster-side (Kubernetes API
server) audit data (FR-008). MUST NOT expose export/retention-config UI (non-goals).

**Scale/Scope**: Bounded to the most recent 500 entries (research.md R3); no pagination or
server-side filtering needed at this scale.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design below.*

- **I. Operator-First, Explicit Mutation** — PASS. This feature adds no new mutating action; it
  only observes/records the outcome of existing, already-confirmed mutations. The viewer itself
  is read-only (no confirmation dialogs needed, nothing to mutate).
- **II. One Contract, Backend Owns the Cluster** — PASS. All persistence (`audit-log.json`) lives
  in the Rust backend (`src/backend/`); the UI only ever reads via the new
  `RefreshAuditLog` protocol command and never touches `~/.fathom/` directly. New types are
  defined once in `src/protocol/v1/audit.rs` and consumed via generated TS bindings.
- **III. Tested at the Boundaries** — PASS (planned). New Playwright E2E scenario (User Story 1 +
  one filter check per the task brief), new command/event round-trip serialization test, new
  Rust unit tests for `audit_summary()` and the retention-truncation logic, frontend unit tests
  for filter/search logic.
- **IV. Density Scales With Width** — PASS (planned). The new pane follows the existing
  mobile-first, panel-priority layout conventions already used by other read-only list panes
  (e.g. Events); no horizontal scroll; column priority for narrow widths (drop context/request-id
  columns first, keep action/target/outcome/timestamp).
- **V. Fast and Quiet** — PASS. Bounded 500-entry in-memory list; no polling/streaming added (the
  pane re-fetches on open/manual refresh, not continuously); idle CPU impact is zero (no new
  background task).
- **VI. User Data Is Sacred** — PASS. New file only; a missing/corrupt `audit-log.json` yields an
  empty history and is never destructively overwritten before being readable (same
  "unreadable → defaults, file untouched" pattern as `load_ui_config`/`load_cluster_settings`).
  No secrets are ever written to audit entries (target names/namespaces only — same class of data
  already shown elsewhere in the UI, never credential/secret contents).

**Result**: No violations. No Complexity Tracking entries needed.

## Project Structure

### Documentation (this feature)

```text
specs/026-mutation-audit-log-viewer/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── protocol-v1-audit-log.md
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

Existing single-project layout (Tauri desktop app: Rust backend at repo root `src/`, frontend
under `ui/`). This feature adds files within that existing structure; it does not introduce a new
top-level project.

```text
src/
├── protocol/v1/
│   ├── audit.rs                 # NEW: AuditLogEntry, AuditActionKind, AuditOutcome
│   ├── cmd.rs                   # + RefreshAuditLog variant
│   ├── event.rs                 # + AuditLogLoaded variant (or reuse an existing list event)
│   ├── mutation.rs               # + audit_summary() classification (mirrors mutation_target())
│   └── route.rs                  # + SectionKey::audit_log()
├── backend/
│   ├── audit_log.rs              # NEW: load/save/append/truncate ~/.fathom/audit-log.json
│   ├── persistence.rs             # + audit_log_path() helper, consistent with existing *_path() fns
│   └── cmd/                       # + handler for RefreshAuditLog (wherever read-only list
│                                    #   commands are dispatched, e.g. alongside events/pods)
└── backend.rs                     # run_cmd: capture audit_summary() before dispatch_cmd moves
                                     # cmd; write audit entry after ok/error is known

ui/src/
├── protocol/generated/            # regenerated bindings for the new types/command/event
├── components/audit/              # NEW: AuditLogPane, filter/search controls, entry list/table
├── store/handlers/                # + handler for AuditLogLoaded, + filter/search state slice
└── layout/ or components/nav/     # + nav entry to open the Audit Log pane

tests/                              # Rust integration tests (existing tests/ dir)
└── audit_log_*.rs                  # NEW: round-trip + retention truncation tests

ui/tests/e2e/
└── audit-log.spec.ts               # NEW: Playwright — perform mutation, view entry, filter
```

**Structure Decision**: Single existing project (Rust backend + `ui/` frontend, Tauri v2 desktop
app). No new project/module boundary; all additions are new files within the existing
`src/protocol/v1/`, `src/backend/`, and `ui/src/` trees, following established naming and module
conventions (mirrors how spec 014's `team.rs`/`team_store.rs` and spec 011's RBAC additions were
structured).

## Complexity Tracking

*No violations — table omitted.*

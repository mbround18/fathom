# Implementation Plan: Keel run approval

**Branch**: `064-keel-run-approval` | **Date**: 2026-10-31 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/064-keel-run-approval/spec.md`

## Summary

Keel holds every run under the catch-all policy, but nothing can approve, deny or expire it. This
plan:

- adds Approve and Deny for `pending_approval` runs. Both are preview and confirm with step-up, on
  the existing `approvals` table (kind `approval`), and are decided in one locked transaction;
- copies each held run's terms (approvals needed, four-eyes, window) onto the run row, so a later
  policy change can't move a waiting run;
- adds a 30 s expiry sweeper that also catches up on start;
- puts the rules in pure keel-core functions;
- on the web, adds an approval panel on the run page, a "Waiting for approval" filter, and a
  neutral count chip on the Runs nav item;
- adds a new `runs` guide.

## Technical Context

**Language/Version**:

- Rust (edition 2024, latest stable) for `crates/keel-core`, `crates/keel-protocol` and
  `apps/keel/server`.
- TypeScript 6 and React 19, built with Vite, for `apps/keel/web`.
- Markdown for `docs/guides/keel`.

**Primary Dependencies**: existing only.

- Server: axum, sqlx (PostgreSQL), tokio.
- Protocol: ts-rs, serde.
- Web: `@companion/ui`, zustand.
- **No new dependencies.**

**Storage**: PostgreSQL. One additive migration, `0013_run_approval.sql`:

- three columns on `runs`;
- one column on `approvals`;
- a partial index;
- a backfill.

See [data-model.md](data-model.md).

**Testing**:

- `cargo test -p keel-core approval`: the rules.
- `cargo test -p keel-server --test run_approval`: Postgres-backed. It covers:
  - the decisions;
  - four-eyes;
  - duplicates;
  - races;
  - expiry and catch-up;
  - frozen terms;
  - audit rows.
- Vitest: `canApprove`, `approvalView`, the approvals store.
- Playwright: a new project, `s064` (`tests/e2e/s064/`), against the fake backend.
  `polish-screens.spec.ts` gains the panel at 360 and 1280 px.

**Target Platform**:

- The Keel server is a Linux container.
- The web app runs in evergreen browsers, from 360 px up.

**Project Type**: The existing Keel server, protocol and web app (spec 055).

**Performance Goals**:

- The sweeper's update uses the partial index, so each tick is cheap.
- The count query touches only `pending_approval` rows.
- The count refetch is debounced by 500 ms.

**Constraints**:

- Exactly one decision per run (SC-004).
- Expiry happens within 60 s, across restarts (SC-002).
- Signed-mode review is untouched (FR-023).
- Policy authoring is untouched (FR-024).
- No host changes (FR-025).
- The accent is reserved for primary actions and the current nav item.
- No horizontal scroll at 360 px.

**Scale/Scope**:

- 4 stories, 25 FRs.
- About 2 new server functions, 1 sweeper, 3 protocol commands and 1 event, 1 migration.
- On the web: 3 new components or modules and 1 guide.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | How this plan complies |
|---|---|
| I. Operator-first, explicit mutation | Approve and Deny are preview and confirm, with step-up. The preview states the effect. Nothing approves implicitly. |
| II. One contract per app | All changes are additive in `keel-protocol`. The web app uses the ts-rs output, and the server owns the rules (keel-core). |
| III. Tested at the boundaries | Unit tests cover the keel-core rules. A Postgres integration test covers the server. A Playwright project covers each story, and the layout guard is extended. |
| IV. Density scales with width | The panel and filter are checked at 360 and 1280 px, in light and dark. |
| V. Fast and quiet | One partial-index sweep every 30 s. No polling in the browser. The count rides on existing `RunState` events. |
| VI. User data is sacred | The migration is additive. `approvals` stays insert-only. Audit entries are insert-only. Runs are never deleted, only moved to a terminal state. |

**Result**: pass. Nothing goes in Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/064-keel-run-approval/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── protocol.md
│   └── ui.md
└── tasks.md            # /speckit-tasks
```

### Source Code

```text
crates/keel-core/src/approval.rs            # HeldTerms, check_approve, after_approve, Refusal (replaces placeholder)
crates/keel-protocol/src/cmd.rs             # PreviewRunApprove, PreviewRunDeny, CountPendingApprovals
crates/keel-protocol/src/run.rs             # RunApproval, ApprovalEntry, ApprovalReason; RunInfo.approval
crates/keel-protocol/src/event.rs           # PendingApprovals, WorkspaceCount
apps/keel/server/migrations/0013_run_approval.sql
apps/keel/server/src/runs.rs                # hold terms in create_run; list ordering; RunInfo.approval
apps/keel/server/src/approvals.rs           # new: previews, approve/deny tx, count, sweeper
apps/keel/server/src/review.rs              # overridden(): set held terms when still pending
apps/keel/server/src/main.rs                # spawn the approval sweeper
apps/keel/server/tests/run_approval.rs
apps/keel/web/src/protocol/generated/*      # regenerated
apps/keel/web/src/features/hosts/workspaces.ts   # canApprove
apps/keel/web/src/features/runs/approval.ts      # approvalView
apps/keel/web/src/features/runs/ApprovalPanel.tsx
apps/keel/web/src/features/runs/approvals-store.ts
apps/keel/web/src/features/runs/RunPage.tsx, RunsPage.tsx, store.ts
apps/keel/web/src/app/Shell.tsx, nav.ts     # Runs count chip
apps/keel/web/src/help/guides.ts, glossary.ts    # 'runs' guide slug, links
apps/keel/web/tests/support/fake-backend.ts # approve/deny/count handlers
apps/keel/web/tests/unit/{approval,approvals-store,workspaces}.test.ts
apps/keel/web/tests/e2e/s064/*.spec.ts      # + playwright.config.ts project s064
docs/guides/keel/runs.md (new), getting-started.md, README.md
```

**Structure Decision**: the existing layout. Rules go in keel-core, the wire in keel-protocol, IO
in keel-server, and the screens in the web app's `features/runs`. This follows the agnostic core
plus adapters split already in use.

## Complexity Tracking

None.

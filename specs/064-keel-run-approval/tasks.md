---

description: "Task list for spec 064, Keel run approval"
---

# Tasks: Keel run approval

**Input**: `specs/064-keel-run-approval/`: plan.md, spec.md, research.md, data-model.md,
contracts/protocol.md, contracts/ui.md and quickstart.md.

**Tests**: these are required (Principle III, plan.md "Testing"). Write each story's tests first.

**Paths**:

- `server/` means `apps/keel/server/`.
- `web/` means `apps/keel/web/`.
- `core/` means `crates/keel-core/src/`.
- `proto/` means `crates/keel-protocol/src/`.

**Gates for agents**:

- Run targeted checks only, one at a time, logging to files, with `CARGO_BUILD_JOBS=4`.
- Never run `scripts/ci.sh`, `paws`, `dagger` or the full Playwright suite.
- Write Playwright as `--project=<name>`, with the `=`.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup

- [ ] T001 Add Playwright project `s064` to `web/playwright.config.ts`, with
  `testDir: './tests/e2e/s064'` and Desktop Chrome. Create `web/tests/e2e/s064/`. Never name a spec
  there `enroll.spec.ts` or `settings-*.spec.ts`.
- [ ] T002 [P] Create `server/tests/run_approval.rs` as an empty harness on `tests/common`, like
  `tests/runs.rs`. It should seed:
  - a workspace;
  - an owner;
  - two approvers;
  - an operator;
  - a viewer;
  - a binding and job with one host;
  - helpers to start a held run, both by hand (`requested_by` set) and by push (`requested_by`
    null).

---

## Phase 2: Foundational (blocks every story)

- [ ] T003 [P] Write unit tests in `core/approval.rs` (`#[cfg(test)]`). They should cover:
  - `check_approve` refuses:
    - `NotPending` for every state except `pending_approval`;
    - `Expired` when `now >= expires_at`;
    - `FourEyesRequester` only when `four_eyes` holds and the actor equals `Some(requester)`. A
      `None` requester is never refused;
    - `AlreadyAnswered`.
  - `after_approve` returns `Some(RunEvent::Approved)` exactly when `approvals >= needed`.
  - Each `Refusal::code()` and `message()` matches the data-model.md table verbatim.
- [ ] T004 Implement `core/approval.rs`, replacing the placeholder:
  - `HeldTerms { needed: u16, four_eyes: bool, expires_at: i64 }`;
  - `Refusal { NotPending(RunState), Expired, FourEyesRequester, AlreadyAnswered }`, with
    `code()` and `message()`;
  - `check_approve(...)`, `check_deny(...)` (it skips four-eyes), and `after_approve(...)`.

  Make T003 pass with `cargo test -p keel-core approval`.
- [ ] T005 Write migration `server/migrations/0013_run_approval.sql`, per data-model.md:
  - **On `runs`**:
    - add `approvals_needed integer check (approvals_needed >= 1)`, `four_eyes boolean` and
      `approval_expires_at timestamptz`;
    - backfill the `pending_approval` rows from their policy, or else
      `1 / false / created_at + interval '24 hours'`, using `created_at + approval_ttl`;
    - then add
      `check (state <> 'pending_approval' or (approvals_needed is not null and approval_expires_at
      is not null))`;
    - add the index `runs_pending_approval on runs (approval_expires_at) where state =
      'pending_approval'`.
  - **On `approvals`**: add `approver_role text`, and
    `revoke update, delete, truncate on approvals from keel_app`. First check that the code never
    updates or deletes `approvals`; if it does, keep only what is safe and note it.
  - Check `server/tests/migrate.rs` still passes.
- [ ] T006 [P] Add the protocol types, per contracts/protocol.md:
  - in `proto/run.rs`: `ApprovalReason` (1–500 characters, trimmed, validated like
    `ReviewComment`), `RunApproval`, `ApprovalEntry`, and
    `RunInfo.approval: Option<RunApproval>` with `#[serde(default)]`;
  - in `proto/cmd.rs`: `PreviewRunApprove { run }`, `PreviewRunDeny { run, reason }` and
    `CountPendingApprovals`, with action names `preview_run_approve`, `preview_run_deny` and
    `count_pending_approvals`;
  - in `proto/event.rs`: `PendingApprovals { counts: Vec<WorkspaceCount> }`.

  Regenerate the ts-rs output into `web/src/protocol/generated/` the way the repo already does.
  Find out how by grepping for `export_to` or the generator script.
- [ ] T007 Hold the terms when a run enters `pending_approval`:
  - in `server/src/runs.rs` `create_run`, set `approvals_needed`, `four_eyes` and
    `approval_expires_at = now() + ttl` from the resolved policy. A nil or synthetic policy uses
    the catch-all terms;
  - in `server/src/review.rs` `overridden()`, when the next state is `pending_approval`, set the
    same three columns from the run's policy, or the catch-all if it is gone.

  Add a test to T002's file showing that editing and removing the policy after the hold leaves the
  run's columns unchanged (FR-009).
- [ ] T008 Fill `RunInfo.approval` in `server/src/runs.rs` (`RunRow`, `list_runs`, `get_run`) for
  runs whose `approvals_needed` is not null:
  - `approvals`: `kind = 'approval'`, `decision = 'approve'`, joined to `users` for the display
    name, with "Deleted user" if the user is gone;
  - `denial`: the `deny` row.
  - Timestamps are RFC 3339, via the existing `governance::ts`.
- [ ] T009 [P] Add `canApprove(me, ws)` to `web/src/features/hosts/workspaces.ts`, mirroring
  `crates/keel-core/src/authz.rs`: Approver, WsAdmin, instance owner and admin. Add its cases to
  `web/tests/unit/workspaces.test.ts`.
- [ ] T010 [P] Write `web/tests/unit/approval.test.ts`, then implement
  `web/src/features/runs/approval.ts` `approvalView({ run, me })`. It returns:
  - `act`;
  - `requester` (a four-eyes run, and `me` is `requested_by`);
  - `answered` (`me` is in `approvals`);
  - `restricted`;
  - `closed` (the state is not `pending_approval`).
- [ ] T011 Extend `web/tests/support/fake-backend.ts`. It needs:
  - held runs, with `approval` set;
  - `preview_run_approve` and `preview_run_deny`, with the preview text, step-up, and confirm,
    which replies `Run` and pushes `RunState`;
  - the refusal codes from data-model.md;
  - `count_pending_approvals`;
  - `ListRuns { state: 'pending_approval' }`, oldest first.

**Checkpoint**: core, migration, protocol and web helpers are in place. The stories can start.

---

## Phase 3: User Story 1, an approver lets a held run go ahead (P1) 🎯 MVP

**Goal**: approving a held run, with step-up, queues it once it has enough approvals.

**Independent Test**: with one approval needed, an approver approves from Runs → the run is
queued → it runs. The audit log has `run_approved` and `run_approval_queued`.

### Tests for US1

- [ ] T012 [P] [US1] In `server/tests/run_approval.rs`, cover:
  - approve 1 of 1 → `queued`, with `queued_at` set, and the dispatcher kicked;
  - 2 of 2: the first approval keeps it `pending_approval`, and a second, different approver
    queues it;
  - a four-eyes requester → `four_eyes_requester`, audited as `run_approval_refused`;
  - a four-eyes push run (no requester) → any approver may approve;
  - the same approver twice → `already_answered`, counted once;
  - an operator or viewer → `forbidden`;
  - confirm without step-up → `step_up_required`, with no row written;
  - the audit rows and their contexts, per data-model.md.
- [ ] T013 [P] [US1] Write `web/tests/e2e/s064/approve.spec.ts`:
  - **As an approver**: open a held run, and check the panel shows the job, inventory, hosts,
    commit, "Needs 1 approval" and when the window closes. Then Approve → the preview → confirm
    (fake step-up) → the run shows Queued.
  - **On a 2-approval run**: check it shows `data-approval-count="1/2"` after one approval.
  - **As the four-eyes requester**: Deny only, plus the "Someone else must approve this run." line.

### Implementation for US1

- [ ] T014 [US1] Create `server/src/approvals.rs`, and register it in `lib.rs` and in the command
  router where `PreviewCancel` is routed. It provides:
  - **`preview_run_approve`**:
    - `require(Approve)`;
    - pre-check with `core::approval`;
    - return a `Draft` with `step_up: true`, naming the job, inventory, hosts, a short commit and
      the effect ("queues the run" or "adds approval k+1 of N").
  - **A pending op that rechecks, then calls `approve()`**, which runs one transaction:
    1. lock the run with `select … for update`;
    2. if the window has passed, expire the run in this transaction, commit, audit and refuse;
    3. `check_approve`;
    4. insert into `approvals`: `kind 'approval'`, `decision 'approve'`, `step_up_at now()`, and
       `approver_role` (the caller's effective role);
    5. count;
    6. `after_approve` → transition, setting `queued_at`;
    7. audit `run_approved`, plus `run_approval_queued` when the run was queued.

    A unique violation maps to `already_answered`. Every refusal is audited as
    `run_approval_refused` with `Decision::Denied`. After commit: `queue::push_state`, and
    `queue::kick` if the run was queued. Reply `get_run`.

  Make T012 pass.
- [ ] T015 [US1] Build `web/src/features/runs/ApprovalPanel.tsx` per contracts/ui.md, and render it
  in `RunPage.tsx` above the hosts when `run.approval` is set. It needs:
  - the terms, "k of N", the approvals list and the window close time (relative and absolute);
  - Approve (primary), through the existing preview and confirm hook used by Cancel;
  - the `data-*` hooks from contracts/ui.md.
  - Run the impeccable skill over the panel.

  Make T013's approve cases pass.

**Checkpoint**: US1 works alone: the 063 walk-through is no longer blocked.

---

## Phase 4: User Story 2, an approver stops a held run (P1)

**Goal**: one denial ends a held run as denied, with an optional reason, and it never reaches a
host.

**Independent Test**: deny with a reason → the run shows `denied` / "Approval denied" and the
reason. No `run_hosts` row passes `cancelled`, and the audit log has `run_denied`.

### Tests for US2

- [ ] T016 [P] [US2] In `server/tests/run_approval.rs`, cover:
  - deny with a reason → `denied` / `approval_denied`, the queued `run_hosts` `cancelled`, and the
    reason stored and shown in `RunInfo.approval.denial`;
  - deny after 1 of 2 approvals → `denied`;
  - a four-eyes requester may deny;
  - a 501-character reason is refused by the protocol;
  - a race: `tokio::join!` an approve (as the last approval) and a deny on the same run, 20 times
    → each time exactly one decided state, the other refused `run_answered`, and both audited
    (SC-004).
- [ ] T017 [P] [US2] Write `web/tests/e2e/s064/deny.spec.ts`:
  - Deny with a reason → the run shows Approval denied and the reason;
  - the reason field counts up to 500;
  - a refusal from the server shows its message in the dialog.

### Implementation for US2

- [ ] T018 [US2] In `server/src/approvals.rs`, add `preview_run_deny` and `deny()`:
  - the same lock, expiry and state checks as approve;
  - `check_deny` (no four-eyes);
  - insert `decision 'deny'` with `comment` set to the reason;
  - `RunEvent::Denied`;
  - cancel the queued `run_hosts`;
  - audit `run_denied` with the reason;
  - `push_state`.

  Make T016 pass.
- [ ] T019 [US2] Add Deny (destructive, with an optional "Reason" textarea, `maxLength` 500, and a
  counter) to `ApprovalPanel.tsx`, and show the denial in the panel's history. Make T017 pass.

---

## Phase 5: User Story 3, held runs don't wait forever (P2)

**Goal**: an unanswered held run expires within a minute of its window, and catches up after
restarts.

**Independent Test**: a policy with a short TTL; wait past it → the run is `expired`, the audit log
has `run_approval_expired` by Keel, and a later approve is refused `approval_expired`.

### Tests for US3

- [ ] T020 [P] [US3] In `server/tests/run_approval.rs`, cover:
  - one sweeper tick, with the run's `approval_expires_at` set in the past (an SQL update in the
    test) → `expired` / `approval_expired`, `ended_at` set, and a `System` audit entry;
  - approve after expiry and before a tick → refused `approval_expired`, and the run is now
    `expired`;
  - start-up catch-up: start the sweeper on a database with an overdue run → it expires on the
    first tick, without waiting a period;
  - `needs_review` runs and `review_groups` are untouched (FR-023).
- [ ] T021 [P] [US3] Extend `web/tests/e2e/s064/approve.spec.ts`: with the panel open, the fake
  backend pushes `RunState expired` → the page shows Approval expired and the buttons go away.
  Approving a run the fake marks expired shows "This run's approval window has closed."

### Implementation for US3

- [ ] T022 [US3] In `server/src/approvals.rs`, add `pub fn spawn_approval_sweeper(app: AppState,
  period: Duration) -> JoinHandle<()>`:
  - sweep once immediately, then every `period`;
  - each sweep runs the guarded `update … returning`, then audits each run as
    `run_approval_expired` with `Actor::System`, then calls `push_state`.

  Spawn it in `server/src/main.rs` next to the session sweeper, with a 30 s period, and abort it on
  shutdown. Expose the sweep function to tests. Make T020 pass.

---

## Phase 6: User Story 4, approvers see what waits for them (P2)

**Goal**: a live count on the Runs nav item, and a "Waiting for approval" filter. People who can't
approve see who can, and no buttons.

**Independent Test**: as an approver, two held runs → nav chip **2**, and the filter lists both,
oldest first. As a viewer → "An approver or workspace admin can approve this." and no buttons.

### Tests for US4

- [ ] T023 [P] [US4] In `server/tests/run_approval.rs`, cover `CountPendingApprovals`:
  - it counts only workspaces where the caller has `Approve`;
  - it excludes runs the caller answered;
  - it excludes four-eyes runs the caller started;
  - a viewer gets an empty list.

  Also check that `ListRuns { state: pending_approval }` comes back oldest first.
- [ ] T024 [P] [US4] Write `web/tests/unit/approvals-store.test.ts`: refetch on connect, debounce
  `RunState` events by 500 ms, and a total of 0 hides the chip.
- [ ] T025 [P] [US4] Write `web/tests/e2e/s064/waiting.spec.ts`:
  - the approver's nav chip `[data-nav-badge="runs"]` reads 2, then 1 after one approval, then is
    gone;
  - the filter `?state=pending_approval` lists the held runs oldest first;
  - an empty filter shows NoMatch;
  - the viewer and operator see the `restricted` panel, with no `[data-approve]` or `[data-deny]`;
  - the chip is not the accent colour: compare its computed color with the current nav link's, as
    in the 063 nav test.

### Implementation for US4

- [ ] T026 [US4] Server work:
  - in `server/src/approvals.rs`, `count_pending_approvals`, one query grouped by workspace and
    filtered to `Approve` workspaces;
  - in `server/src/runs.rs` `list_runs`, `order by created_at asc` when `state = pending_approval`.

  Make T023 pass.
- [ ] T027 [US4] Implement `web/src/features/runs/approvals-store.ts` (zustand), `useApprovals()`
  per contracts/ui.md. Wire it into `web/src/app/connection.ts`, or wherever `RunState` events
  fan out. Make T024 pass.
- [ ] T028 [US4] Show the count chip on the Runs nav item in `web/src/app/Shell.tsx`, through an
  optional per-section badge in `web/src/app/nav.ts`:
  - neutral `bg-muted`;
  - `aria-label` "{n} runs waiting for your approval".
- [ ] T029 [US4] Add the "All / Waiting for approval" filter to `web/src/features/runs/RunsPage.tsx`
  (`data-runs-filter`, synced to `?state=`):
  - the held row's state cell reads "Waiting · k of N";
  - an empty filtered list shows `NoMatch`.
  - Show the `restricted` and `requester` variants in `ApprovalPanel`.

  Make T025 pass.

---

## Phase 7: Polish & cross-cutting

- [X] T030 [P] Write `docs/guides/keel/runs.md`, covering:
  - run states;
  - how approval policies hold a run;
  - approving;
  - denying;
  - four-eyes;
  - the approval window and expiry;
  - who can approve;
  - the waiting count.

  Add it to `docs/guides/keel/README.md`. Add `'runs'` to `GuideSlug` in `web/src/help/guides.ts`
  (and the test's list).
- [X] T031 [P] In `docs/guides/keel/getting-started.md`, replace the note that runs can't be
  approved in the web app with the real steps (FR-020), linking to `runs.md#approving-a-run`.
- [X] T032 Link the Runs empty state and the run-related glossary entries
  (`web/src/help/glossary.ts`) to the `runs` guide (FR-022). `guide-links.test.ts` must pass.
- [ ] T033 Extend `web/tests/e2e/polish-screens.spec.ts` with a held run's panel (the act and
  restricted variants), and the Runs nav chip. Check 360 and 1280 px, light and dark.
- [ ] T034 Targeted checks, one at a time:
  - `cargo clippy -p keel-core -p keel-protocol -p keel-server --all-targets -- -D warnings`;
  - `cargo fmt --check`;
  - web typecheck, lint and Vitest;
  - `--project=s064`, `--project=s063` and `--project=s060`.
- [ ] T035 The main session runs `scripts/ci.sh` once on the combined branch, and reports the
  timing (the wall time, the slowest steps, and the delta against the last pass).
- [ ] T036 With the owner: the quickstart manual walk-through, which also closes 063 T053.

---

## Dependencies

- Setup (T001–T002) → Foundational (T003–T011) → the stories.
- **US1** (T012–T015) and **US2** (T016–T019) both build on `approvals.rs`.
  - US2's server work follows T014, because it edits the same file.
  - The web parts (T015, T019) touch one file, so they run in order.
- **US3** (T020–T022) needs only the Foundational phase plus `approvals.rs`. It can run after T014.
- **US4** (T023–T029) needs US1's panel (T015) for the variants.
- **Polish** comes after all the stories. T030–T032 can start any time after T006.

## Parallel examples

- **Foundational**: T003/T004 (core), T005 (migration), T006 (protocol), T009/T010 (web helpers).
- **Agent split**:
  - Agent A, the server: T002, T005, T007, T008, T012, T014, T016, T018, T020, T022, T023, T026.
  - Agent B, the web: T001, T009–T011, T013, T015, T017, T019, T021, T024, T025, T027–T029, T033.
  - Agent C: docs, T030–T032.
  - Core and protocol (T003, T004, T006) land first, in the main session or with Agent A, because
    both A and B depend on the generated types.

## Implementation strategy

1. The MVP is US1 plus US2 (both P1). This unblocks the 063 walk-through.
2. Then US3 (expiry), so held runs can't pile up.
3. Then US4 (visibility).
4. Then polish and the docs.
5. One CI run at the end. Brochure screenshots (063 T054) remain the last step of the batch.

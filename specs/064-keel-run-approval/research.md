# Research: Keel run approval

No NEEDS CLARIFICATION remained after the spec. These are the decisions the plan rests on, checked
against the code on 2026-10-31.

## What exists today

- `keel-core/src/run.rs`: the state machine already has `pending_approval --Approved--> queued`
  and `--Denied/Expired--> denied/expired`.
  - `StateReason` already has `approval_denied` and `approval_expired`.
  - Nothing in keel-server sends `Approved` or `Expired` for a policy hold.
- `keel-core/src/approval.rs` is an empty placeholder from spec 055 Phase 1.
- Migration `0007_signed.sql` already created `approvals`:
  - its `kind` column takes `'approval' | 'override'`, and its `decision` column takes
    `'approve' | 'deny'`;
  - it has `comment`, `at` and `step_up_at`;
  - it has `unique nulls not distinct (run_id, approver_user_id, kind, review_group_id)`.
  - Only `override` rows are written today.
- `runs.policy_id` is `on delete set null`, and `runs.requested_by` is nullable (push runs).
- `review.rs` is the model to follow:
  - `preview_override_deny` builds a `Draft` and a pending op, and rechecks the capability at
    confirm;
  - `deny()` takes a transaction, inserts into `approvals`, transitions, audits, commits, and then
    `queue::push_state`.
- `preview.rs`: `Draft { step_up: true }` makes `Confirm` require a recent passkey step-up (spec
  054). This is all FR-003 needs.
- `audit::Actor::System` exists for entries with no person behind them (expiry).
- `main.rs` already runs a periodic task, the session sweeper. Expiry follows the same pattern.

## R1. A held run keeps its terms on the run row

- **Decision**: add three columns to `runs`:
  - `approvals_needed integer`;
  - `four_eyes boolean`;
  - `approval_expires_at timestamptz`.

  They are set when the run enters `pending_approval`: in `create_run`, and in `review::overridden`
  when an override leaves the run still needing approval.
  - The window is counted from the moment the run starts waiting.
  - A run whose policy is gone uses the catch-all's terms (one approval, no four-eyes, 24 h). This
    matches `Policy::catch_all`, and the fail-closed resolution in `policy.rs`.
  - The migration backfills existing `pending_approval` runs:
    - from their policy, if it still exists;
    - otherwise, from catch-all defaults;
    - with `approval_expires_at = created_at + ttl`.

    So the first sweep after upgrade expires runs that have long been stuck (FR-011).
- **Rationale**: FR-009 says a later policy edit or removal must not change a waiting run. Copying
  the terms is the only way that holds after `policy_id` is set to null.
- **Alternatives considered**: a policy-version table. Rejected: it is heavier, and only these three
  values matter.

## R2. Approvals reuse the `approvals` table

- **Decision**: write `kind = 'approval'` rows.
  - Approve: `decision = 'approve'`, `step_up_at = now()`.
  - Deny: `decision = 'deny'`, with `comment` holding the optional reason.
  - The protocol caps the reason at 500 characters (the table allows 2000).
  - Add `approver_role text`, holding the caller's effective role in the workspace at that moment
    (edge case "loses their role").
  - The existing unique constraint gives at most one row per person per run (FR-006). A person who
    approved cannot later deny the same run. The refusal says so: `already_answered`, "You have
    already answered this run."
- **Rationale**: the table was designed for this, and kept insert-only (Principle VI). No new table
  is needed.
- **Alternatives considered**: a separate `run_approvals` table. Rejected: it would duplicate a
  table built for the purpose.

## R3. One transaction decides, under a row lock

- **Decision**: approve and deny both run in one transaction:
  1. `select … from runs where id = $1 for update`;
  2. check that the state is `pending_approval`, and that `approval_expires_at > now()`. If the
     window has passed, expire the run here, in the same transaction, then refuse;
  3. apply four-eyes;
  4. insert the approval;
  5. count distinct `approve` rows of kind `approval`;
  6. transition with `keel_core::run::transition`;
  7. audit;
  8. commit;
  9. `queue::push_state`, then `queue::kick` when the run is queued.

  A racing answer waits on the lock, then sees a state other than `pending_approval` and is refused
  with `run_answered` ("This run has already been answered; it is now {state}."). Refusals are
  audited outside the transaction, with `Decision::Denied` (FR-008, FR-013).
- **Rationale**: SC-004 asks for exactly one decision. A row lock is the simplest serialization,
  and `cancel_run` already relies on `locked_update_run`.

## R4. The decision logic lives in keel-core

- **Decision**: replace the placeholder `keel-core/src/approval.rs` with pure functions:
  - `HeldTerms { needed: u16, four_eyes: bool, expires_at: i64 }`;
  - `check_approve(terms, now, actor, requester, already_answered) -> Result<(), Refusal>`;
  - `after_approve(terms, approvals) -> RunEvent?`, which returns `Approved` once the count reaches
    `needed`;
  - `Refusal`: `Expired | FourEyesRequester | AlreadyAnswered | NotPending(state)`, each with an
    `OpError` code and message.

  The server supplies the facts it reads from the database.
- **Rationale**: Principle III. Every rule in FR-004 to FR-007 is unit-testable without Postgres.

## R5. Expiry is a sweeper, plus a check at decision time

- **Decision**: spawn `runs::spawn_approval_sweeper(app, period)` in `main.rs`, next to the session
  sweeper.
  - The period is 30 s (FR-012: within one minute). It is configurable for tests.
  - It sweeps once immediately on start (FR-011).
  - Each tick runs
    `update runs set state = 'expired', state_reason = 'approval_expired', ended_at = now()
    where state = 'pending_approval' and approval_expires_at <= now()
    returning id, workspace_id, policy_id`.
  - It then writes one `run_approval_expired` audit entry per run with `Actor::System`, and calls
    `push_state` for each.
  - The decision path (R3) also refuses an expired run, so a slow tick never lets one through.
  - Signed-mode `review_groups` expiry is not touched (FR-023).
- **Rationale**: SC-002 holds across restarts, with no new dependency.

## R6. The protocol is additive

- **Decision**:
  - New commands:
    - `PreviewRunApprove { run }`;
    - `PreviewRunDeny { run, reason: Option<ApprovalReason> }`, where `ApprovalReason` is 1–500
      characters.
    - Both return a `Preview` with `step_up: true`, and `Confirm` replies `Run`.
  - `RunInfo` gains `approval: Option<RunApproval>`, filled for runs that have ever been held. It
    holds needed, four-eyes, expiry, the approvals so far and any denial.
  - New `CountPendingApprovals` → `Event::PendingApprovals { counts: Vec<WorkspaceCount> }`. A run
    counts only in workspaces where the caller has `Approve`, and only if the caller hasn't
    answered it and isn't excluded by four-eyes.
  - `ListRuns { state: pending_approval }` already filters. When that filter is set, the server
    orders oldest first (FR-017).
  - The TypeScript types are regenerated by ts-rs into `web/src/protocol/generated`.
- **Rationale**: Principle II: one contract, which the backend owns. Old clients ignore the new
  field.

## R7. The waiting count stays live without a new channel

- **Decision**: the web app keeps a small `approvals` store.
  - It calls `CountPendingApprovals` on connect, and on every `RunState` event that enters or
    leaves `pending_approval`. Events are debounced by 500 ms.
  - The Runs nav item shows the total as a neutral count chip, `bg-muted`, never the accent.
  - The count hides at 0, and for people with no `Approve` anywhere.
- **Rationale**: FR-018 and SC-005 are met through the existing live events. There is no polling
  and no new push type.

## R8. The UI reuses the preview dialog

- **Decision**:
  - `RunPage` gains an `ApprovalPanel`, shown when the run is `pending_approval`, or ended with an
    approval reason. It shows the terms, the approvals so far, the time the window closes, and:
    - **Approve** (primary) and **Deny** (destructive, with an optional reason field), when
      `canApprove(me, ws)`;
    - for the person who started a four-eyes run, Deny only, plus the line "Someone else must
      approve this run.";
    - otherwise, "An approver or workspace admin can approve this.", with no buttons.
  - Both buttons go through the existing preview and confirm flow, which already handles step-up.
  - `RunsPage` gains a state filter, "All" or "Waiting for approval". A held row shows "1 of 2
    approvals".
  - The Runs empty state and the glossary's run terms link to the new `runs` guide (FR-022).
  - `canApprove(me, ws)` sits next to `canOperate` in `features/hosts/workspaces.ts`, and mirrors
    `authz.rs`: Approver, WsAdmin, owner and admin.

## R9. Tests

- **keel-core**: unit tests for every rule in `approval.rs`. The existing state-machine table test
  stays as it is.
- **keel-server**: `tests/run_approval.rs`, against Postgres like `tests/runs.rs`:
  - approve one of one;
  - two of two, by different people;
  - a four-eyes requester is refused;
  - a duplicate is refused;
  - deny with a reason;
  - a racing approve and deny (two tasks, one decided);
  - expiry by the sweeper, and catch-up on start;
  - a policy edited or removed after the hold keeps the terms;
  - the audit rows for each case.
- **Web**:
  - Vitest for `canApprove`, the count store and `ApprovalPanel`'s variants.
  - A Playwright project, `s064` (`tests/e2e/s064/`), against the fake backend:
    - approve, deny, four-eyes, restricted;
    - the nav count;
    - the filter.
  - `polish-screens` gains the approval panel at 360 and 1280 px.

# Data model: Keel run approval

Migration `apps/keel/server/migrations/0013_run_approval.sql`. It is additive only: nothing earlier
is dropped or narrowed.

## `runs`: new columns

| Column | Type | Rule |
|---|---|---|
| `approvals_needed` | `integer` | `>= 1`. Set when the run enters `pending_approval`. Null for a run that was never held. |
| `four_eyes` | `boolean` | Set alongside `approvals_needed`. |
| `approval_expires_at` | `timestamptz` | When the run started waiting, plus the policy's TTL. |

- **Constraint**: `check (state <> 'pending_approval' or (approvals_needed is not null and
  approval_expires_at is not null))`.
- **Index**: `runs_pending_approval on runs (approval_expires_at) where state =
  'pending_approval'`. It serves the sweeper and the count.
- **Backfill**: for rows already `pending_approval`:
  - take the terms from `approval_policies` where `policy_id` still matches;
  - otherwise use 1, false and 24 h;
  - the expiry is `created_at + ttl`.

## `approvals`: one new column

| Column | Type | Rule |
|---|---|---|
| `approver_role` | `text` | The approver's effective workspace role when they answered: `approver`, `ws_admin`, `owner` or `admin`. Null on rows written before this migration. |

- `kind = 'approval'` rows: `decision` is `approve` or `deny`, and `comment` holds the deny reason
  (at most 500 characters, enforced by the protocol). `step_up_at` is set on both decisions.
- The existing `unique nulls not distinct (run_id, approver_user_id, kind, review_group_id)` means
  one answer per person per run.
- The table stays insert-only for `keel_app` (Principle VI). Add
  `revoke update, delete on approvals from keel_app` if it is not already revoked.

## Held terms (keel-core `approval::HeldTerms`)

```text
HeldTerms { needed: u16 (>= 1), four_eyes: bool, expires_at: unix seconds }
```

These are read from the run row, never from the live policy (FR-009).

## Rules (keel-core `approval`)

| Check | Refusal code | Message |
|---|---|---|
| The run's state is not `pending_approval` | `run_answered` | "This run has already been answered; it is now {state}." |
| `now >= expires_at` | `approval_expired` | "This run's approval window has closed." |
| Four-eyes, and the caller is `requested_by` (Approve only) | `four_eyes_requester` | "You started this run, so someone else must approve it." |
| The caller already has an approval row for this run | `already_answered` | "You have already answered this run." |
| The caller lacks `Approve` in the workspace | the existing `forbidden` | unchanged |

## Transitions

- **Approve** → count distinct `approve` rows of kind `approval`. If the count is at least
  `needed`, send `RunEvent::Approved`, which moves the run to `queued` and sets `queued_at`.
  Otherwise the run stays `pending_approval`.
- **Deny** → `RunEvent::Denied` moves the run to `denied` (`approval_denied`), and its queued
  `run_hosts` become `cancelled`.
- **The sweeper, or a decision made after the window** → `RunEvent::Expired` moves the run to
  `expired` (`approval_expired`).

## Audit actions

| Action | Actor | Decision | Context |
|---|---|---|---|
| `run_approved` | the user | allowed | policy, role, `approvals` (count), `needed` |
| `run_approval_queued` | the user who gave the last approval | allowed | policy, `approvals` |
| `run_denied` | the user | allowed | policy, role, reason |
| `run_approval_refused` | the user | denied | refusal code, state |
| `run_approval_expired` | `System` | allowed | policy, `expires_at` |

Every entry has the target `{ "run": id }`, and is written to the run's workspace (FR-014).

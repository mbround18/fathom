# Data model: Keel host provisioning and convergence (spec 068)

Migration `apps/keel/server/migrations/0018_converge.sql`.

**Numbering risk.** 0015 (`0015_join.sql`, 067 D3) and 0017 (`0017_relays.sql`, 067) are
reserved; 0016 is applied. If another branch lands 0018 first, rename this file **before** any
shared database applies it (sqlx stores the checksum and version of applied migrations). The
merge step checks `ls migrations | sort` for duplicates.

Grants follow the existing pattern: `keel_app` gets `select, insert, update`; no `delete`.

## Altered tables

### `runs`

| Column | Change | Notes |
|---|---|---|
| `kind` | check gains `converge` | Parent row. `job_id` null, `job_name` null. |
| `parent_run_id` | new `uuid null → runs(id)` | Set on a converge's child runs. Index `(parent_run_id)`. |
| `check_mode` | new `boolean not null default false` | Copied to children. |
| `retry_of` | new `uuid null → runs(id)` | On a converge created by Retry. |
| `state` | check gains `waiting_checks` | Before `queued`/`needs_review` when the binding has required checks (US7). |
| `state_reason` | check gains `checks_failed`, `checks_timeout`, `replaced`, `runner_too_old` | `replaced`: an unapproved converge superseded by a newer one for the same host. |

Check `runs_converge_shape`: `kind = 'converge'` ⇒ `job_id is null and parent_run_id is null`;
`parent_run_id is not null` ⇒ `kind = 'normal'`.

### `run_hosts`

| Column | Change | Notes |
|---|---|---|
| `state` | check gains `blocked`, `skipped`, `covered` | `blocked`: a converge step waiting on approval or an earlier step. |
| `covered_by` | new `uuid null → runs(id)` | The converge that covered this host's part (US4). |
| `skip_reason` | new `text null` | `dependency_failed` \| `converge_cancelled`. |
| refusal codes | gain `runner_too_old`, `choice_mismatch`, `env_refused` | Runner refusals. |

Terminal states are now `succeeded, failed, refused, lost, cancelled, skipped, covered`.
Non-terminal: `blocked, queued, sent, running`.

### `hosts`

No change. `allowed_profiles` (ceiling) and `host_profiles` (assignment) keep their meaning.

### `host_enrollments`

| Column | Change | Notes |
|---|---|---|
| `auto_converge` | new `boolean not null default true` | Enrollment checkbox (US2). |

### `job_policies` / policy JSON

`PolicyMatch` gains `check: Option<bool>` in JSON; no column change (policies are stored as
jsonb).

## New tables

### `converge_hosts`

| Column | Type | Notes |
|---|---|---|
| `run_id` | `uuid` → `runs(id)` | The converge (parent). |
| `host_id` | `uuid` → `hosts(id)` | |
| `state` | `text not null` | `waiting` \| `holding` \| `succeeded` \| `failed` \| `cancelled` |
| `profiles` | `text[] not null` | The host's assigned profiles at plan time. |
| `started_at`, `ended_at` | `timestamptz null` | |

PK `(run_id, host_id)`.
**Lock**: `create unique index converge_hosts_one_holding on converge_hosts (host_id) where state = 'holding';`
Index `(host_id) where state in ('waiting','holding')` for the coverage lookup.

### `converge_steps`

| Column | Type | Notes |
|---|---|---|
| `run_id` | `uuid` → `runs(id)` | Parent. |
| `host_id` | `uuid` → `hosts(id)` | |
| `position` | `smallint not null` | 0-based, per host. |
| `job_name` | `text not null` | |
| `step_run_id` | `uuid not null` → `runs(id)` | The child run for this job. Several hosts share one child run. |
| `via` | `text[] not null` | Profile path that put the step there, e.g. `{keel-node,debian}` (shown in the preview). |

PK `(run_id, host_id, position)`; unique `(run_id, host_id, job_name)`. The step's state is the
`run_hosts` row `(step_run_id, host_id)`; no duplicate state column.

### `profile_suggestions`

| Column | Type | Notes |
|---|---|---|
| `host_id` | `uuid` → `hosts(id)` | |
| `profile` | `text not null` | |
| `action` | `text not null` | `add` \| `remove` |
| `groups` | `text[] not null` | The inventory groups that caused it. |
| `binding_id` | `uuid` → `bindings(id)` | |
| `commit` | `text not null` | Manifest commit it was computed at. |
| `first_seen_at`, `last_seen_at` | `timestamptz not null` | |
| `dismissed_at`, `dismissed_by` | null | A dismissed suggestion returns only if its `groups` change. |

PK `(host_id, profile, action)`. Rows not produced by the latest sync are deleted in the same
transaction (`keel_app` gets `delete` on this table only).

### `binding_required_checks`

| Column | Type | Notes |
|---|---|---|
| `binding_id` | `uuid` → `bindings(id)` | |
| `context` | `text not null` | Forge status context, e.g. `ci/lint`. ≤ 200 chars. |
| `created_at`, `created_by` | | |

PK `(binding_id, context)`. `bindings` gains `checks_timeout_secs integer not null default 3600`
(60–86400).

### `run_check_states`

| Column | Type | Notes |
|---|---|---|
| `run_id` | `uuid` → `runs(id)` | |
| `context` | `text not null` | |
| `state` | `text not null` | `pending` \| `success` \| `failure` \| `error` \| `missing` |
| `target_url` | `text null` | Forge link, shown in the UI. |
| `updated_at` | `timestamptz` | |

## Trigger: `converge_advance`

`after update of state on run_hosts for each row when (old.state in
('blocked','queued','sent','running') and new.state in
('succeeded','failed','refused','lost','cancelled','skipped','covered'))`:

1. Find `converge_steps` row by `(step_run_id = new.run_id, host_id = new.host_id)`. None ⇒
   return (not a converge step).
2. If `new.state = 'succeeded'`: the next position's `run_hosts` row `blocked → queued`. If there
   is none, `converge_hosts` → `succeeded`, `ended_at = now()`.
3. Else: every later position's row `blocked → skipped` (`skip_reason = 'dependency_failed'`, or
   `converge_cancelled` when `new.state = 'cancelled'`), and `converge_hosts` →
   `failed`/`cancelled`.
4. The child runs' and parent run's terminal state is derived by `try_finish` as today (the
   parent finishes when every `converge_hosts` row is terminal).

The `skipped` updates in step 3 re-enter the trigger with `old.state = 'blocked'`; step 1 finds
their row, and step 3 has nothing later left to skip, so recursion ends.

## State transitions

Converge host:

```text
waiting ──first step claimed (dispatch, same tx)──► holding
holding ──last step succeeded──► succeeded
holding ──step failed/lost/refused──► failed
waiting|holding ──deny/expire/cancel/replaced──► cancelled
```

Step (`run_hosts` of a child run):

```text
blocked ──parent approved (first step) or previous step succeeded──► queued ──► sent ──► running ──► succeeded|failed|lost
blocked ──previous step not succeeded──► skipped
blocked|queued ──cancel──► cancelled
```

Path run host under coverage: `(insert) covered` with `covered_by`.

## Validation rules

- Recipe expansion ≤ 32 steps per host; ≤ 64 profiles; recipe depth ≤ 8.
- `env` names: `^[A-Z_][A-Z0-9_]{0,63}$`, not on the shared refusal list; ≤ 16 per job.
- `choice` values `^[A-Za-z0-9][A-Za-z0-9._-]{0,62}$`; a `from:` directory yields ≤ 256 entries.
- Required check contexts: ≤ 16 per binding.

## Audit events

`converge.create` (hosts, steps, commit, inventory revision, check, actor — `system:enroll` on
enrollment), `converge.approve` (existing approval entries, on the parent), `converge.cancel`,
`converge.replace`, `converge.retry`, `run_host.covered` (run, host, covered_by),
`host.profiles_set` (adds, removes, step-up id), `profile_suggestion.dismiss`,
`binding.required_checks_set`, `run.checks_failed`. Never credential values.

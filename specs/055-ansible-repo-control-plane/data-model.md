# Data model: Keel (055)

PostgreSQL tables (R10). All ids are UUIDv7 unless noted, and all times are `timestamptz` in UTC.
"Audited" means that every insert, and every update to the fields named, writes an
`audit_event` row in the same transaction.

## Identity and access

**users**: `id`, `issuer`, `subject` (unique together), `display_name`, `email?`,
`instance_role` (`owner | admin | user`), `authz_epoch bigint`, `disabled_at?`, `created_at`.
- At least one owner always exists; removing or demoting the last owner is refused.
- Any narrowing change increments `authz_epoch` (R11).
- Audited.

**sessions**: `id` (random 256-bit, stored hashed), `user_id`, `authz_epoch`, `verified_at?`
(passkey), `created_at`, `last_seen_at`, `expires_at`.
- A session is valid only while `authz_epoch = users.authz_epoch`.

**passkeys**, **passkey_recovery**: owned by `crates/web-passkeys` (R11). The shape matches 054,
moved from files to tables.

**workspaces**: `id`, `slug` (unique, `^[a-z0-9][a-z0-9-]{0,62}$`), `name`, `created_at`,
`archived_at?`.
- A workspace is the authorization boundary. Projects inside it group bindings and jobs for
  display only.

**projects**: `id`, `workspace_id`, `slug` (unique per workspace), `name`.

**teams**: `id`, `name`, `origin` (`local | suite`).

**team_members**: `team_id`, `user_id`, `origin`.

**grants**: `id`, `team_id`, `workspace_id`, `role` (`viewer | operator | approver | ws_admin`),
`origin` (`local | suite`), `bundle_version?`.
- Suite-origin rows are written only by the suite adapter, and are read-only in the UI and the
  protocol.
- Effective access is the union of grants, minus any suite suspension (057 FR-039).
- Audited.

**suspensions** (suite only): `issuer`, `subject`, `bundle_version`, `until?`.

**api_tokens**: `id`, `name`, `owner_user_id`, `workspace_id`, `actions text[]`, `hash`,
`expires_at`, `last_used_at?`, `revoked_at?`.
- A token acts as its owner, narrowed to `actions` in one workspace, for the public API (R14).
- Audited.

## Repositories and receivers

**repo_bindings**: `id`, `project_id`, `url`, `branch`, `transport` (`https | ssh`),
`credential_id?` (a deploy key or token, from **credentials**), `known_host?` (pinned host key for
SSH), `allow_force_push bool` (default false), `poll_interval?`, `status` (`active | disabled`),
`last_commit?`, `last_synced_at?`, `last_sync_error?`.
- Unique on (`url`, `branch`, `project_id`).
- Disabling a binding cancels its queued and pending runs (R7).
- Audited.

**receivers**: `id`, `name`, `provider` (`github | gitlab | forgejo | generic`),
`secret_current` (sealed), `secret_previous?` (sealed), `previous_expires_at?`,
`status` (`active | disabled`), `created_at`, `rotated_at?`.
- Secrets are never returned after creation; the creation response shows the secret once.
- Audited, without secrets.

**receiver_bindings**: `receiver_id`, `binding_id`, `created_by`, `created_at`.
- Many-to-many. A delivery is accepted only when its receiver is active, bound to an active
  binding whose URL and branch match the payload (FR-005).
- Audited.

**deliveries**: `id`, `receiver_id`, `provider_delivery_id`, `received_at`, `binding_id?`,
`decision` (`accepted | refused`), `refusal?`, `sync_state`, `commit?`.
- `refusal` is one of `bad_signature`, `unknown_receiver`, `receiver_disabled`,
  `no_matching_binding`, `binding_disabled`, `ref_not_branch` or `payload_invalid`.
- `sync_state` is one of `syncing`, `synced`, `duplicate`, `stale`, `history_rewritten` or
  `sync_failed`.
- Unique on (`receiver_id`, `provider_delivery_id`). Refusals are audited (FR-006).

## Inventory

**inventories**: `id`, `workspace_id`, `name` (unique per workspace), `source` (`repo | managed`),
`binding_id?` and `path?` (when `repo`), `var_schema? jsonb`, `status` (`active | deprecated`).

**inventory_revisions**: `id`, `inventory_id`, `seq` (1, 2, … per inventory), `parent_id?`,
`content jsonb` (normalized hosts, groups and vars), `content_hash`, `source_commit?`,
`author_user_id?`, `restored_from?` (rollback, R8), `imported_from?` (migration provenance),
`status` (`valid | invalid`), `errors jsonb`, `warnings jsonb`, `created_at`.
- Revisions are immutable: no `UPDATE` grant.
- A new revision is created only when `content_hash` changes.

**inventory_hosts** (derived per revision, for search and target resolution): `revision_id`,
`host`, `groups text[]`.

## Credentials

**credentials**: `id`, `workspace_id`, `name` (unique per workspace), `kind`
(`ssh_key | become_password | vault_password | env | git_token`), `sealed bytea?`,
`wrapped_key bytea?`, `fingerprint`, `external_ref?` (`ballast:<ns>/<name>`, suite only),
`created_by`, `rotated_at?`.
- Exactly one of `sealed` or `external_ref` is set.
- `fingerprint` is SHA-256 over the plaintext, truncated, and shown so operators can tell keys
  apart.
- Audited, without values.

**credential_grants**: `credential_id`, `binding_id`.
- A manifest's credential name resolves only through a grant (R6).

## Jobs, policies and runs

**jobs** (synced from `.keel/jobs.yaml` per binding commit): `id`, `binding_id`, `name`,
`definition jsonb`, `defined_at_commit`, `removed_at_commit?`.
- `definition` holds the playbook, inventory, limit, vars, credential names, path filters and
  rollback (contracts/repo-manifest.md).

**approval_policies**: `id`, `workspace_id`, `priority`, `match jsonb`, `requires_approval bool`,
`approvals_needed int` (1 or more), `four_eyes bool`, `approval_ttl interval`.
- `match` selects by job name glob, inventory and limit glob.
- The policy with the lowest `priority` among those that match wins.
- Every workspace has a priority-`max` catch-all `{requires_approval: true}`, which cannot be
  deleted (R6).
- Audited.

**runs**: `id`, `workspace_id`, `job_id`, `kind` (`normal | rollback`), `reverses_run_id?`,
`trigger` (`delivery | poll | manual | api | workflow`), `delivery_id?`, `requested_by?`, `commit`,
`inventory_revision_id`, `resolved_hosts text[]`, `limit`, `policy_id`, `state`,
`state_reason?`, `runner_id?`, `queued_at?`, `started_at?`, `ended_at?`, `summary jsonb?`.
- `state` and its transitions are below.
- `summary` holds Ansible's per-host stats: `ok`, `changed`, `failed`, `unreachable`, `skipped`,
  `rescued` and `ignored`, plus host counts.
- Creation pins `commit` (`refs/keel/runs/<id>`) and `inventory_revision_id`.

```
pending_approval ──approve(n of N)──▶ queued ──claim──▶ running ──▶ succeeded | failed
       │  └──deny──▶ denied              │                  └─cancel─▶ cancelled
       └──ttl──▶ expired                 └─cancel / binding disabled─▶ cancelled
(policy says no approval) ─────────────▶ queued
```

- `state_reason` is one of `approval_denied`, `approval_expired`, `binding_disabled`,
  `cancelled_by_user`, `runner_lost`, `runner_tainted`, `inventory_invalid`, `artifact_missing`
  or `credential_unavailable`.
- Every transition is audited.

**approvals**: `run_id`, `approver_user_id`, `decision` (`approve | deny`), `comment?`, `at`,
`step_up_at`.
- Unique on (`run_id`, `approver_user_id`).
- An approval made after `queued_at + approval_ttl` is refused (R7).

**run_host_results**: `run_id`, `host`, `ok`, `changed`, `failed`, `unreachable`, `skipped`,
`rescued`, `ignored`, `first_failure jsonb?`.
- `first_failure` holds the task, module, message and return code, so operators have actionable
  context (US2 scenario 3).

**run_events**: `run_id`, `seq`, `at`, `kind`, `host?`, `task?`, `data jsonb`.
- Batched inserts, from the `keel_events` stream (R3).

**run_log_chunks**: `run_id`, `seq`, `stream` (`stdout | stderr`), `bytes`.
- Bounded per run (default 64 MiB). Past the bound, Keel keeps the head and tail and records a
  truncation marker.

## Runners

**runner_pools**: `id`, `name`, `shared bool`.

**runner_pool_workspaces**: `pool_id`, `workspace_id`.
- A non-shared pool has exactly one workspace (R2).

**runners**: `id`, `pool_id`, `name`, `key_fingerprint`, `version`, `ansible_version`,
`state` (`idle | busy | tainted | offline`), `last_seen_at`.

## Migration

**instance**: a singleton holding `instance_id` and the Ed25519 `instance_key` (sealed).

**trusted_instances**: `instance_id`, `public_key`, `trusted_by`, `trusted_at`.

**migrations**: `id`, `direction` (`export | import`), `bundle_sha256`, `source_instance_id`,
`dest_instance_id`, `actor`, `plan jsonb` (diff and conflict choices), `state`
(`planned | applied | refused`), `at`.

## Audit

**audit_event**: `id` (UUIDv7, stable for suite forwarding), `at`, `actor` (issuer, subject and
display name, or `system`, `receiver:<id>` or `runner:<id>`), `action`, `workspace_id?`,
`target jsonb`, `decision` (`allowed | denied`), `reason?`, `context jsonb`, `prev_hash`, `hash`.
- `context` holds the binding, commit, inventory revision, run and hosts.
- Insert-only, enforced by grant and trigger. `hash` is SHA-256(prev_hash ‖ canonical row) (R10).
- Indexed on (`at`), (`workspace_id`, `at`), (`actor`, `at`) and (`action`, `at`), so the
  searches in FR-014 are served from indexes.

**audit_outbox** (suite only): `event_id`, `attempts`, `next_attempt_at`.

## Suite

**suite_enrollment**: a singleton holding `portal_url`, `portal_key`, `contract_version`,
`bundle_version`, `bundle_received_at`, `max_age` and `state`.

**scope_mappings**: `suite_scope`, `workspace_id`, `project_id?`.

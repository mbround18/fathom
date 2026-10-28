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

**teams**: `id`, `name`, `origin` (`local` only in 055; `suite` arrives with the integration spec, R14).

**team_members**: `team_id`, `user_id`, `origin`.

**grants**: `id`, `team_id`, `workspace_id`, `role` (`viewer | operator | approver | ws_admin`),
`profiles text[]` (empty means every profile, R21), `origin` (`local` in 055).
- Effective access is the union of grants. A scoped grant covers a host only if the host has one
  of its profiles; an action on several hosts needs coverage of all of them.
- Audited.

**api_tokens**: `id`, `name`, `owner_user_id`, `workspace_id`, `actions text[]`, `hash`,
`expires_at`, `last_used_at?`, `revoked_at?`.
- `actions` may only name commands on the token allow-list (`keel_protocol::TOKEN_ACTIONS`),
  checked at creation and on every request. Tokens are read-only: the list holds reads, a write
  joins it only by a deliberate choice, and passkey and token management never do.
- A token acts as its owner, narrowed to `actions` in one workspace, for the public API (R14).
- Audited.

## Repositories and receivers

**repo_bindings**: `id`, `project_id`, `url`, `branch`, `transport` (`https | ssh`),
`credential_id?` (a deploy key or token, from **credentials**), `known_host?` (pinned host key for
SSH), `allow_force_push bool` (default false), `poll_interval?`, `signed_mode bool` (default
false, R19), `override_quorum int` (default 1, R20), `status` (`active | disabled`),
`last_commit?`, `last_synced_at?`, `last_sync_error?`.
- Turning signed mode off is audited and needs step-up. A binding in signed mode may only use
  `repo` inventories.
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
(`vault_password | env | secret_vars | git_https_token | git_ssh_key`, R12), `sealed bytea?`,
`wrapped_key bytea?`, `fingerprint`, `created_by`, `rotated_at?`.
- `sealed` is always set in 055. Ballast references (`external_ref`) arrive with the integration
  spec (R14).
- `fingerprint` is SHA-256 over the plaintext, truncated, and shown so operators can tell keys
  apart.
- Audited, without values.

**credential_grants**: `credential_id`, `binding_id`, `profiles text[]` (empty means every
profile).
- A manifest's credential name resolves only through a grant (R6). A host receives a credential
  only if the grant covers one of its profiles. `git_*` kinds are never sent to hosts.

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

**runs**: `id`, `workspace_id`, `job_id`, `kind` (`normal | rollback | baseline`),
`reverses_run_id?`, `trigger` (`delivery | poll | manual | api | workflow`), `delivery_id?`,
`requested_by?`, `commit`, `inventory_revision_id`, `resolved_hosts text[]`, `limit`,
`policy_id`, `signed_mode bool`, `state`, `state_reason?`, `queued_at?`, `started_at?`,
`ended_at?`, `summary jsonb?`.
- `state` and its transitions are below.
- `summary` holds Ansible's per-host stats: `ok`, `changed`, `failed`, `unreachable`, `skipped`,
  `rescued` and `ignored`, plus host counts.
- Creation pins `commit` (`refs/keel/runs/<id>`) and `inventory_revision_id`.

```
needs_review ──override(quorum)──┐          (signed mode, a commit in some host's range fails)
     └─deny / ttl─▶ denied | expired
                                 ▼
pending_approval ──approve(n of N)──▶ queued ──dispatch──▶ running ──▶ succeeded | failed | refused
       │  └──deny──▶ denied              │                  └─cancel─▶ cancelled
       └──ttl──▶ expired                 └─cancel / binding disabled─▶ cancelled
(policy says no approval, all ranges signed) ─▶ queued
```

- An override counts as the run's approval when the same people satisfy the approval policy.
- The run's state summarizes its hosts (`run_hosts`): `succeeded` only if every host succeeded;
  `refused` if any host's own verification refused it.
- `state_reason` is one of `approval_denied`, `approval_expired`, `binding_disabled`,
  `cancelled_by_user`, `runner_lost`, `inventory_invalid`, `artifact_missing`,
  `credential_unavailable` or `host_refused`.
- Every transition is audited.

**approvals**: `run_id`, `approver_user_id`, `kind` (`approval | override`),
`decision` (`approve | deny`), `comment?`, `at`, `step_up_at`, `review_group_id?`,
`challenge_sha256?`, `assertion jsonb?` (credential id, authenticator data, client data JSON,
signature), `expires_at?`.
- Unique on (`run_id`, `approver_user_id`, `kind`, `review_group_id`).
- An approval made after `queued_at + approval_ttl` is refused (R7).
- `override` rows require a comment and an assertion over the canonical challenge (R20). They are
  sent to hosts unchanged; the host verifies them itself.

**review_groups**: `id`, `run_id`, `base_commit?` (null for a baseline), `target_commit`,
`hosts text[]`, `commits jsonb` (id, author, committer, signature status and signer per commit),
`flags jsonb` (trust or targeting changes, R20), `state` (`open | approved | denied | expired`).
- One per distinct host base in a signed-mode run that fails the signature check.

**run_hosts**: `run_id`, `host_id`, `base_commit?`, `state` (`queued | sent | running | succeeded |
failed | refused | lost | cancelled`), `refusal?` (`unsigned`, `bad_override`, `base_mismatch`,
`profile_not_allowed`, `downgrade`, `replay`, `ledger_corrupt`, `version_too_old`),
`ledger_hash?`, `started_at?`, `ended_at?`.

**run_host_results**: `run_id`, `host_id`, `ok`, `changed`, `failed`, `unreachable`, `skipped`,
`rescued`, `ignored`, `first_failure jsonb?`.
- `first_failure` holds the task, module, message and return code, so operators have actionable
  context (US2 scenario 3).

**run_events**: `run_id`, `seq`, `at`, `kind`, `host?`, `task?`, `data jsonb`.
- Batched inserts, from the `keel_events` stream (R3).

**run_log_chunks**: `run_id`, `seq`, `stream` (`stdout | stderr`), `bytes`.
- Bounded per run (default 64 MiB). Past the bound, Keel keeps the head and tail and records a
  truncation marker.

## Hosts, profiles and trust

**hosts** (one per enrolled host runner, R1): `id`, `workspace_id`, `inventory_name` (unique per
workspace), `ssh_key_fingerprint` (SHA-256 of the enrolled Ed25519 key, unique), `ssh_public_key`,
`allowed_profiles text[]` (as reported by the host's trust policy), `require_signed bool`
(reported), `runner_version`, `ansible_version`, `state` (`online | offline | revoked`),
`enrolled_at`, `last_seen_at`, `revoked_at?`.
- Revoking a host adds its certificate serials to the KRL and closes its session. Audited.

**host_enrollments**: `id`, `workspace_id`, `inventory_name`, `profiles text[]`, `token_hash`,
`expires_at`, `used_at?`, `host_id?`, `created_by`.
- Single-use, with a 15-minute TTL (R17). Audited.

**join_signing_key** (one row, contracts/join.md): `kid`, sealed Ed25519 private key, `public_key`,
`created_at`, `retired_at?`. Rotating it retires the row and every join token signed by it.

**join_tokens**: `jti` (pk), `workspace_id`, `kid`, `token_hash` (SHA-256 of the whole token),
`inventory_name?`, `profiles text[]`, `expires_at`, `created_by`, `created_at`, `retired_at?`.
- At most one row per workspace with `retired_at` null (partial unique index). Rotate and disable
  set `retired_at`. Audited by `jti`; the token itself is never stored.

**join_requests**: `id`, `workspace_id`, `jti`, `ssh_key_fingerprint`, `ssh_public_key`,
`inventory_name` (suggested), `allowed_profiles text[]`, `require_signed`, `hostname`, `os`, `arch`,
`runner_version` (all reported, length-bounded), `source_addr`, `state`
(`pending | accepted | rejected | expired`), `reason?`, `first_seen_at`, `last_seen_at`,
`decided_by?`, `decided_at?`, `host_id?`.
- Unique per (`workspace_id`, `ssh_key_fingerprint`). Pending rows expire after 7 days; rejected
  fingerprints are answered `JoinRejected` for 30 days. Accepting creates the `hosts` row.

**profiles**: `id`, `workspace_id`, `name` (unique per workspace, `^[a-z0-9][a-z0-9-]{0,62}$`),
`description?`, `declared_at_commit?` (from `.keel/jobs.yaml`).

**host_profiles**: `host_id`, `profile_id`, `assigned_by`, `assigned_at`.
- Must be a subset of `hosts.allowed_profiles`; anything else is refused (R21). Audited.

**ssh_cas**: `id`, `kind` (`host | runner`), `public_key`, `sealed_private_key`, `state`
(`active | retiring | retired`), `created_at`.
- The host CA signs the listener's host certificate; the runner CA signs runner certificates.
  Rotation keeps the retiring CA trusted until its certificates expire.

**ssh_certs**: `serial bigint` (unique), `host_id`, `principals text[]`, `valid_after`,
`valid_before`, `revoked_at?`.
- The KRL is built from rows with `revoked_at` set and `valid_before` in the future.

**trusted_signers**: `id`, `workspace_id`, `kind` (`openpgp | ssh`), `fingerprint` (unique per
workspace), `public_key`, `label`, `user_id?` (for four-eyes), `added_by`, `added_at`,
`removed_at?`.
- Keel's list is used for the early check only. Hosts enforce their own pinned list (R19).
  Changes need step-up and are audited.

**host_revisions**: `host_id`, `job_id`, `commit`, `run_id`, `authorization` (`signed |
override | baseline`), `signers text[]`, `approval_ids uuid[]`, `ledger_hash`, `applied_at`,
`outcome`.
- One row per host run, reported by the host (R19). The latest row per (`host_id`, `job_id`) is
  the host's base. Drift is the number of commits from it to the binding's `last_commit`.
- Insert-only.

## Migration

**instance**: a singleton holding `instance_id` and the Ed25519 `instance_key` (sealed).

**trusted_instances**: `instance_id`, `public_key`, `trusted_by`, `trusted_at`.

**migrations**: `id`, `direction` (`export | import`), `bundle_sha256`, `source_instance_id`,
`dest_instance_id`, `actor`, `plan jsonb` (diff and conflict choices), `state`
(`planned | applied | refused`), `at`.

## Audit

**audit_event**: `id` (UUIDv7, stable for suite forwarding), `at`, `actor` (issuer, subject and
display name, or `system`, `receiver:<id>` or `host:<id>`), `action`, `workspace_id?`,
`target jsonb`, `decision` (`allowed | denied`), `reason?`, `context jsonb`, `prev_hash`, `hash`.
- `context` holds the binding, commit, inventory revision, run and hosts.
- Insert-only, enforced by grant and trigger. `hash` is SHA-256(prev_hash ‖ canonical row) (R10).
- Indexed on (`at`), (`workspace_id`, `at`), (`actor`, `at`) and (`action`, `at`), so the
  searches in FR-014 are served from indexes.

## Suite (deferred)

`suspensions`, `audit_outbox`, `suite_enrollment` and `scope_mappings` belong to the suite
integration spec (research R14). 055 creates none of them.

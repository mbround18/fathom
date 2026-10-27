---

description: "Task list for Keel, the Git-bound Ansible control plane (055)"
---

# Tasks: Keel, Ansible control plane from source control

**Input**: Design documents from `/specs/055-ansible-repo-control-plane/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: REQUIRED. Constitution Principle III is non-negotiable, so:

- every `Cmd` and `Event` variant gets a round-trip test, written with the variant;
- every receiver refusal, state-machine transition and `keel-trust` verdict gets a failing-first
  test row before the code that satisfies it;
- each user story gets Playwright E2E against the fake backend (projects `us1`–`us9`) or a server
  test against a throwaway PostgreSQL with the fake runner.

**Organization**: phases follow dependency order, not the spec's numbering. Priorities are
P1: US1, US4, US6, US7, US8; P2: US2, US5, US9; P3: US3.

1. Setup and Foundational create the crates, the protocol skeleton, the database, audit,
   sign-in, passkeys, Preview/Confirm and the web shell.
2. US6 (teams and grants) comes first, because every later command is authorized through it.
3. US1 (bindings and receivers), then US4 (inventory), then US2 (runs, with the fake runner).
4. US7 (real host runners over SSH), then US8 (signed mode), then US9 (profiles and revisions).
5. US3 (approvals and audit search) and US5 (rollback and migration) close the governance and
   recovery loops.

US3 and US5 are independent of US7–US9 once US2 is done, so they can run as a separate track.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel: different files, and no dependency on an incomplete task.
- **[Story]**: US1–US9, used in the user-story phases only.

## Rules for every task in this file

- **Builds and tests**:
  - Run them with `CARGO_BUILD_JOBS=4 nice`, one heavy command at a time, using
    `cargo check -p <crate>` while iterating.
  - Gate on the targeted tests named in the task and in quickstart.md §1.
- **CI**: subagents never run `scripts/ci.sh`, `paws` or `dagger`. The main session runs the full
  gate once, on the combined branch.
- **Database tests**: use `KEEL_TEST_DATABASE_URL`. Each test creates and drops its own schema.
  When the variable is unset, the test is skipped with a note, never failed.
- **Docker and hosts**: only T133 (opt-in, `KEEL_IT_HOST=1`) and T134 (compose smoke) touch a real
  daemon. They remove what they created, by name (`keel-it-host-<pid>`). Never prune, never remove
  an image, never touch another container or volume.
- **`../ansible` is not edited by any task here.** The `keel_runner` role is spec 058.
- **Protocol changes**: after any edit to `crates/keel-protocol`, run
  `cargo test -p keel-protocol --test export_bindings` and commit the regenerated bindings with
  the change.
- **Never log or audit**: credential values, receiver secrets, enrollment tokens, passkey
  challenges or assertions' private material, recovery codes, or the master key.
- **Fail closed**: every branch that cannot decide refuses. A test that "allows on error" is a bug.
- **Migrations**: additive only. No `DROP`, no column type change, no `UPDATE` grant on
  `audit_event`, `inventory_revisions` or `host_revisions`.
- **Compose**: every service has `restart: ${RESTART_POLICY:-no}`.
- **Commits**: one signed commit per task or tight group of tasks.

---

## Phase 1: Setup

- [X] T001 Create the agnostic crate `crates/keel-core` (`edition = "2024"`,
  `license = "Apache-2.0"`, `publish = false`).
  - Dependencies: `serde`, `serde_json`, `thiserror`, `uuid` (v7), `time`, `sha2`, `globset`.
    No tokio, no sqlx, no gix, no filesystem or network I/O.
  - Empty modules `policy`, `run`, `approval`, `rollback`, `inventory`, `manifest`, `migration`,
    `authz`, `ports` in crates/keel-core/src/lib.rs. The existing `crates/*` member picks it up.
- [X] T002 [P] Create `crates/keel-protocol` with `serde`, `ts-rs` and `keel-core` (types only),
  modules `cmd`, `event`, `types`, `lib` (`PROTOCOL_VERSION = 1`), and the
  `tests/export_bindings.rs` test writing to apps/keel/web/src/protocol/generated/, copying the
  pattern of crates/depths-protocol/tests/export_bindings.rs.
- [X] T003 [P] Create `crates/keel-git` (`gix` with HTTPS and blocking network client features,
  `keel-core`) with modules `mirror`, `pin`, `ancestry`, `pack`, `diff`.
- [X] T004 [P] Create `crates/keel-trust` (pure, no I/O): dependencies `pgp` (rpgp), `ssh-key`
  (with `ed25519`, `p256`), `serde_jcs`, `sha2`, `serde`, `web-passkeys` (verify-only, T023).
  Modules `commit_check`, `challenge`, `override_verify`, `ledger`; empty
  crates/keel-trust/tests/vectors/.
- [X] T005 [P] Create `crates/keel-exec`: `russh`, `ssh-key`, `tokio`, `serde_json`, `keel-trust`.
  Modules `frames`, `codec` (u32 big-endian length prefix, 16 MiB cap), `ca`, `krl`,
  `server_config`, `client_config`, `pack_builder`, `events` (the `keel_events` line parser),
  `fake` (test double behind a `fake` feature).
- [X] T006 [P] Create `crates/web-passkeys` (store trait, ceremonies, recovery codes, pure
  `verify_assertion`) with `webauthn-rs` 0.5, `p256`, `ed25519-dalek`, `sha2`; dev-dependency
  `webauthn-authenticator-rs` (softpasskey).
  - **054 is not implemented yet** (no passkey module exists in apps/depths/server). Build the
    crate from 054's contracts/protocol.md passkey shapes instead of extracting. Add a note to
    specs/054-depths-root-boundary/tasks.md US4 that Depths consumes `crates/web-passkeys` rather
    than adding `webauthn-rs` to the Depths server.
- [X] T007 Create the binary crate `apps/keel/server` (binary `keel`) and add `"apps/keel/server"`
  to the root Cargo.toml `[workspace] members`.
  - Dependencies: `keel-core`, `keel-protocol`, `keel-git`, `keel-exec`, `keel-trust`,
    `web-auth`, `web-passkeys`, `axum`, `tower`, `tokio`, `sqlx` (postgres, runtime-tokio,
    macros, uuid, time, json), `clap`, `chacha20poly1305`, `ed25519-dalek`, `hmac`, `tracing`,
    `opentelemetry` (workspace versions where they exist).
  - Files: `src/main.rs`, `config.rs`, `db.rs`, `audit.rs`, `seal.rs`, `serve.rs`, `ws.rs`,
    `api.rs`, `hooks.rs`, `sync.rs`, `queue.rs`, `dispatch.rs`, `runner_listener.rs`,
    `telemetry.rs`; `migrations/` empty.
- [X] T008 Create `apps/keel/runner` with binaries `keel-runner` and `keel-executor`
  (`src/bin/keel-runner.rs`, `src/bin/keel-executor.rs`) and add it to `[workspace] members`.
  Also add `ansible/callback/keel_events.py`, `ansible/requirements.txt` (empty placeholder) and
  `systemd/` (empty).
- [X] T009 [P] Create `apps/keel/web` as `@companion/keel-web`, copying apps/depths/web's
  package.json scripts, tsconfig set, vite, vitest and eslint configs, and depending on
  `@companion/ui`, `theme`, `branding` and `terminal` (`workspace:*`). Use the `舵` brand from
  packages/branding.
- [X] T010 [P] In apps/keel/web/playwright.config.ts, define
  `STORIES = ['shell', 'us1', 'us2', 'us3', 'us4', 'us5', 'us6', 'us7', 'us8', 'us9']` with
  `autoWorkers()` from ui/tests/support/workers.ts, and a placeholder spec per directory under
  apps/keel/web/tests/e2e/ that only opens the shell.
- [X] T011 [P] Add the new crates' licenses to deny.toml if `cargo deny check licenses` asks
  (rpgp, russh, ssh-key, gix are MIT/Apache-2.0); confirm no LGPL crate (sequoia) is pulled in.

---

## Phase 2: Foundational (blocks every story)

**Purpose**: the core types, the protocol skeleton, storage, audit, sign-in, passkeys,
Preview/Confirm and the web shell that every story uses.

### Tests first

- [X] T012 [P] Round-trip tests in crates/keel-protocol/tests/roundtrip.rs for `Hello`,
  `CmdEnvelope`, `EventEnvelope`, `Preview`, `Confirm { preview_id, step_up }`,
  `Denied { action, reason }`, `OpError { code, message }`, `Revoked`, `GetMe`/`Me`.
- [X] T013 [P] Audit-chain tests in apps/keel/server/tests/audit.rs (database): an insert computes
  `hash = SHA-256(prev_hash ‖ canonical row)`; `UPDATE` and `DELETE` on `audit_event` fail for the
  app role and for the owner (trigger); `keel audit verify` detects an offline edit.
- [X] T014 [P] Seal tests in apps/keel/server/src/seal.rs: round trip under a test master key;
  wrong key fails; a missing or unreadable `--master-key-file` refuses start; `keel keygen` writes
  32 bytes at mode `0400`.
- [X] T015 [P] Preview/Confirm tests in apps/keel/server/tests/preview.rs: a preview is
  single-use, bound to its session, expires after 5 minutes, and a `Confirm` from another session
  is refused.
- [X] T016 [P] Authorization tests in crates/keel-core/src/authz.rs: deny by default; union of
  grants; a scoped grant (`profiles` non-empty) covers a host only if the host has one of its
  profiles; an action on several hosts needs coverage of all of them; the last owner cannot be
  removed or demoted.

### Implementation

- [X] T017 Define the domain types in crates/keel-core: ids (UUIDv7 newtypes), `InstanceRole`
  (`owner | admin | user`), `TeamRole` (`viewer | operator | approver | ws_admin`), `Slug`
  (`^[a-z0-9][a-z0-9-]{0,62}$`), `Origin` (`local` only), and the ports `GitSource`,
  `SecretResolver`, `SuitePolicy` (standalone adapter only: always local) and `Clock` in
  crates/keel-core/src/ports.rs.
- [X] T018 Implement `authz::effective(grants, host_profiles) -> Access` and `authz::require` in
  crates/keel-core/src/authz.rs until T016 passes.
- [X] T019 Implement the run state machine in crates/keel-core/src/run.rs exactly as the diagram
  in data-model.md (`needs_review`, `pending_approval`, `queued`, `running`, `succeeded`,
  `failed`, `refused`, `cancelled`, `denied`, `expired`), with `state_reason` one of
  `approval_denied`, `approval_expired`, `binding_disabled`, `cancelled_by_user`, `runner_lost`,
  `inventory_invalid`, `artifact_missing`, `credential_unavailable`, `host_refused`. Unit tests
  cover every allowed transition and refuse every other one; the run's state summarizes its hosts
  (`succeeded` only if every host succeeded, `refused` if any host refused).
- [X] T020 Server config in apps/keel/server/src/config.rs with every flag and env var of
  contracts/config.md (`--listen` default `0.0.0.0:7830`, `--runner-listen` `0.0.0.0:7831`,
  `--runner-cert-ttl` `24h`, `--runner-max-offline` `30d`, `--signed-mode-default` `false`,
  `--public-url` required, `--database-url` required, `--master-key-file` required,
  `--data-dir` `/var/lib/keel`, `--static-dir` `/usr/share/keel/web`, auth flags (at least one
  required), `--step-up-window` `5m`, `--approval-ttl-default` `24h`, `--run-log-cap` `64MiB`,
  `--otel-endpoint`), plus unit tests for defaults and each required key's error.
- [X] T021 Migration `0001_identity.sql` in apps/keel/server/migrations/: `users`, `sessions`
  (id stored hashed), `passkeys`, `passkey_recovery`, `workspaces`, `projects`, `teams`,
  `team_members`, `grants` (`profiles text[]`, `origin`), `api_tokens`, and `audit_event` with
  its insert-only grant and update/delete trigger and the indexes (`at`), (`workspace_id`, `at`),
  (`actor`, `at`), (`action`, `at`). Wire `sqlx::migrate!` in apps/keel/server/src/db.rs and the
  `keel migrate` subcommand.
- [X] T022 Implement apps/keel/server/src/audit.rs: `audit::record(tx, actor, action, workspace,
  target, decision, reason, context)` inside the caller's transaction, computing the chain; the
  `keel audit verify` subcommand. Pass T013.
- [X] T023 Implement crates/web-passkeys: registration and authentication ceremonies, a
  `PasskeyStore` trait (Keel's sqlx adapter lives in apps/keel/server/src/passkeys.rs), recovery
  codes, and pure `verify_assertion(credential_public_key, alg, authenticator_data,
  client_data_json, signature, expected_challenge, origin, rp_id) -> Result<(), VerifyError>`
  checking `type == "webauthn.get"`, challenge, origin, `rpIdHash`, UP and UV flags, and ES256 or
  EdDSA. Tests with softpasskey and fixed vectors in crates/web-passkeys/tests/.
- [X] T024 Implement apps/keel/server/src/seal.rs (XChaCha20-Poly1305, per-item data key wrapped
  by the master key) and `keel keygen`. Pass T014.
- [X] T025 Sign-in through `web-auth` in apps/keel/server/src/serve.rs, mirroring Depths: OIDC,
  GitHub and Discord providers, sessions with `authz_epoch`, CSRF on the WebSocket upgrade. The
  first user to sign in on an empty instance becomes `owner`.
- [X] T026 The WebSocket (`GET /api/v1/ws`) and `POST /api/v1/cmd` transports in
  apps/keel/server/src/ws.rs and api.rs: one dispatcher, per-command authorization against the
  session's current epoch, `Denied` audited, streaming commands over `/cmd` refused with
  `use_websocket`. On an epoch change, close the user's sockets with `Revoked`.
- [X] T027 The Preview/Confirm framework in apps/keel/server/src/preview.rs: previews stored
  in memory per session with a 5-minute TTL, `step_up` required flag, passkey step-up within
  `--step-up-window`. Pass T015.
- [X] T028 Session and passkey commands (`GetMe`, the passkey commands as in 054) in
  crates/keel-protocol and apps/keel/server/src/passkeys.rs.
- [X] T029 [P] Static web serving from `--static-dir` (never embedded) and OpenTelemetry setup
  in apps/keel/server/src/telemetry.rs, off unless `--otel-endpoint` is set.
- [X] T030 [P] Web shell in apps/keel/web/src: app frame, sign-in redirect, protocol client over
  the socket, passkey enrol and step-up dialogs, `Revoked` handling, and navigation for bindings,
  receivers, inventory, runs, reviews, hosts, teams, audit, migration (empty pages).
- [X] T031 [P] Fake backend in apps/keel/web/tests/support/fake-backend.ts replaying scripted
  `EventEnvelope`s, following apps/depths/web/tests/support/fake-backend.ts; make the `shell`
  Playwright project pass.

**Checkpoint**: `cargo test -p keel-core -p keel-protocol -p web-passkeys` and
`cargo test -p keel-server` (database) pass; the shell signs in against the fake backend.

---

## Phase 3: User Story 6 — Depths-parity team and user governance (P1)

**Goal**: owner/admin/user roles, workspaces, teams, grants (optionally profile-scoped), immediate
revocation, every denial audited.

**Independent test**: configure teams and roles, verify allowed and denied operations by scope,
change a role, and confirm enforcement updates immediately (open sockets get `Revoked`).

### Tests

- [X] T032 [P] [US6] Round-trip tests for the workspace, team, grant and user commands of
  contracts/protocol.md in crates/keel-protocol/tests/roundtrip.rs.
- [X] T033 [P] [US6] Server tests in apps/keel/server/tests/governance.rs: an operator in team A
  acting on team B's workspace is `Denied` and audited (SC-006); removing a user's grant
  increments `authz_epoch` in the same transaction and closes their socket with `Revoked`;
  removing the last owner is refused.
- [X] T034 [P] [US6] Playwright apps/keel/web/tests/e2e/us6/: create a workspace and team, add a
  member and a grant, see the preview list affected users, and see a `Denied` refusal verbatim.

### Implementation

- [X] T035 [US6] Workspace, project, team, member, grant and user commands in
  crates/keel-protocol/src/cmd.rs and event.rs: `ListWorkspaces`,
  `PreviewWorkspaceCreate/Update/Archive`, `ListTeams`, `PreviewTeamCreate`,
  `PreviewTeamMember{Add,Remove}`, `PreviewGrant{Add,Remove}` (with `profiles`), `ListUsers`,
  `PreviewUserRole`, `PreviewUserRemove`.
- [X] T036 [US6] Handlers in apps/keel/server/src/governance.rs: every narrowing change increments
  `authz_epoch`; role changes need step-up; every change audited in its transaction.
- [X] T037 [US6] API tokens in apps/keel/server/src/api.rs: create (shown once, stored hashed),
  scoped to one workspace and `actions`, acting as the owner narrowed; revoke; audited.
- [X] T038 [P] [US6] Web pages apps/keel/web/src/features/teams/ and users/: lists in the Depths
  dense list pattern, mobile-first; grant editor with an optional profile multi-select.

**Checkpoint**: US6 tests and Playwright `us6` pass.

---

## Phase 4: User Story 1 — Bind repository and receiver (P1)

**Goal**: register a repository binding, create receivers, bind them, and accept only deliveries
tied to an active binding; jobs read from `.keel/jobs.yaml`.

**Independent test**: as an admin, create a binding and a receiver, bind them, post a signed push
(accepted, then synced), and see bad signatures, duplicates and unbound repositories refused and
audited.

### Tests

- [ ] T039 [P] [US1] Receiver signature and refusal table in apps/keel/server/tests/hooks.rs, one
  row per provider of contracts/receivers.md (`github`, `gitlab`, `forgejo` with both header
  spellings, `generic`) and one row per check in order: `404 unknown_receiver`,
  `404 receiver_disabled`, `401 bad_signature`, `200 duplicate`, `202 ref_not_branch`,
  `403 no_matching_binding`, `403 binding_disabled`, `202 accepted`. Include the previous secret
  accepted until `previous_expires_at` and refused after; a body over 1 MiB refused; the response
  never echoes the payload.
- [ ] T040 [P] [US1] gix tests in crates/keel-git/tests/mirror.rs against local bare repositories:
  fetch into `<data_dir>/mirrors/<binding id>.git`, no hooks run, `refs/keel/runs/<id>` pins
  survive a force-push and `gc`, ancestry classifies `synced`, `duplicate`, `stale` and
  `history_rewritten`.
- [ ] T041 [P] [US1] Manifest validation tests in crates/keel-core/src/manifest.rs, one per rule
  of contracts/repo-manifest.md: `version: 1` required, unknown keys refused, `..`/absolute paths
  and escaping symlinks refused, `playbook` must exist, `profiles` non-empty and declared, `limit`
  parses, signed mode requires a repository inventory, ungranted credentials make the job
  `blocked (credential_unavailable)`, `vars` keys like `ansible_*password*`,
  `ansible_ssh_private_key*`, `ansible_become_pass*` refused; an invalid manifest leaves the
  previous job set active with `manifest_invalid`.
- [ ] T042 [P] [US1] Round-trip tests for the binding, receiver and delivery commands and the
  `DeliveryReceived` and `BindingSynced` events.
- [ ] T043 [P] [US1] Playwright apps/keel/web/tests/e2e/us1/: create a binding (preview shows the
  test fetch), create a receiver (secret shown once), bind them, and see deliveries with their
  decisions.

### Implementation

- [ ] T044 [US1] Migration `0002_bindings.sql`: `repo_bindings` (unique on (`url`, `branch`,
  `project_id`); `signed_mode bool` default false; `override_quorum int` default 1; `status`
  `active | disabled`), `receivers` (`secret_current`, `secret_previous?` sealed),
  `receiver_bindings`, `deliveries` (unique on (`receiver_id`, `provider_delivery_id`); `decision`
  `accepted | refused`; `refusal` and `sync_state` enums exactly as data-model.md), `credentials`
  and `credential_grants` (`profiles text[]`), `jobs`, `profiles`.
- [ ] T045 [US1] `GitSource` gix adapter in crates/keel-git/src/mirror.rs and pin.rs: HTTPS with a
  token, SSH with a deploy key and the binding's pinned `known_host` (`StrictHostKeyChecking=yes`),
  pins, ancestry. Pass T040.
- [ ] T046 [US1] Manifest parser and validator in crates/keel-core/src/manifest.rs. Pass T041.
- [ ] T047 [US1] Credentials in apps/keel/server/src/credentials.rs: kinds `vault_password | env |
  secret_vars | git_https_token | git_ssh_key`; sealed at rest; `fingerprint` (SHA-256 of the
  plaintext, truncated); values never returned; create, rotate and grant need step-up;
  `PreviewCredentialRemove` refused while a pending or queued run needs it.
- [ ] T048 [US1] Binding commands in apps/keel/server/src/bindings.rs: `PreviewBindingCreate`
  (test fetch; with signed mode, report which recent commits are unsigned), `PreviewBindingUpdate`,
  `PreviewBindingDisable` (lists the runs it will cancel; cancels queued and pending runs with
  `binding_disabled`), `PreviewBindingRemove`, `SyncBinding`, polling for bindings with
  `poll_interval` only (no timers otherwise).
- [ ] T049 [US1] Receiver commands in apps/keel/server/src/receivers.rs: create (secret returned
  once as `ReceiverSecret { url, secret }`), rotate with `previous_valid_for` (default 24 h),
  disable, remove, bind, unbind, `ListDeliveries`. Creation and rotation need step-up.
- [ ] T050 [US1] `POST /hooks/{id}` in apps/keel/server/src/hooks.rs, with the checks in the order
  of contracts/receivers.md, rate limits per receiver and source IP, then an async fetch. Pass
  T039.
- [ ] T051 [US1] Sync in apps/keel/server/src/sync.rs: fetch, classify, read `.keel/jobs.yaml` at
  the fetched head (never the payload's commit), upsert `jobs` and `profiles`, revise `repo`
  inventories (stub until US4), emit `BindingSynced`.
- [ ] T052 [P] [US1] Web pages apps/keel/web/src/features/bindings/ and receivers/.

**Checkpoint**: US1 scenarios in quickstart.md §2 pass; Playwright `us1` passes.

---

## Phase 5: User Story 4 — Provision and manage inventory safely (P1)

**Goal**: repository-backed and managed inventories with immutable, validated revisions.

**Independent test**: create and update an inventory through a repository change, edit a managed
one, see invalid content refused before use, and see lineage on each revision.

### Tests

- [ ] T053 [P] [US4] Inventory parser and validator tests in crates/keel-core/src/inventory.rs:
  YAML and INI with `group_vars/` and `host_vars/`; Ansible name rules; no group cycles; no
  duplicate host identities; JSON Schema on vars; plaintext-secret warning for `*password*`,
  `*token*`, `*secret*` plain strings (vault values allowed); an invalid revision is stored with
  status `invalid` and its errors.
- [ ] T054 [P] [US4] Server tests in apps/keel/server/tests/inventory.rs: a new revision only when
  `content_hash` changes; `PreviewInventoryEdit` from a stale base is refused as `stale_base`;
  revisions cannot be updated (no grant); a deprecated inventory accepts no new runs.
- [ ] T055 [P] [US4] Playwright apps/keel/web/tests/e2e/us4/: edit a managed inventory, see
  validation errors before saving, diff two revisions, restore an earlier one.

### Implementation

- [ ] T056 [US4] Migration `0003_inventory.sql`: `inventories` (`source` `repo | managed`, name
  unique per workspace), `inventory_revisions` (immutable, no `UPDATE` grant; `seq` per
  inventory; `restored_from?`; `imported_from?`), `inventory_hosts`.
- [ ] T057 [US4] Parser and validator in crates/keel-core/src/inventory.rs. Pass T053.
- [ ] T058 [US4] Inventory commands in apps/keel/server/src/inventory.rs: `ListInventories`,
  `GetInventoryRevision`, `DiffInventory`, `PreviewInventoryCreate`, `PreviewInventoryEdit`,
  `PreviewInventoryRestore` (new revision equal to an old one), `PreviewInventoryDeprecate`,
  `ValidateInventory`. Hook `repo` revisions into sync (T051). Pass T054.
- [ ] T059 [P] [US4] Web page apps/keel/web/src/features/inventory/ with the revision list, diff
  and editor (CodeMirror YAML, as Depths).

**Checkpoint**: US4 scenario in quickstart.md §2 passes; Playwright `us4` passes.

---

## Phase 6: User Story 2 — Drive node operations from source control (P2)

**Goal**: a push that touches a job's paths becomes a pinned run that fans out to hosts, with live
logs and per-host results. Execution goes through the `keel-exec` fake runner; real hosts come in
US7.

**Independent test**: a bound repository event starts a run against the hosts of a job; queued,
running and terminal states, per-host outcomes and failure context are visible.

### Tests

- [ ] T060 [P] [US2] Policy tests in crates/keel-core/src/policy.rs: lowest `priority` among
  matching policies wins; `match` by job name glob, inventory and limit glob; the priority-`max`
  catch-all `{requires_approval: true}` exists in every workspace and cannot be deleted;
  `ExplainPolicy` says which policy wins and why.
- [X] T061 [P] [US2] Frame round trips in crates/keel-exec/tests/frames.rs for every frame of
  contracts/runner.md, plus the codec refusing frames over 16 MiB.
- [X] T062 [P] [US2] `keel_events` parser tests in crates/keel-exec/src/events.rs over recorded
  lines (`playbook_start`, `play_start`, `task_start`, per-host `ok/changed/failed/unreachable/
  skipped`, `stats`), and a Python unit test for apps/keel/runner/ansible/callback/keel_events.py
  that `no_log` results are written only as `censored`.
- [ ] T063 [P] [US2] Server tests in apps/keel/server/tests/runs.rs with the fake runner: a push
  that touches a job's `paths` creates a run pinned to the fetched commit and inventory revision;
  a force-push gives `history_rewritten` and no run; per-host results arrive within 30 s of the
  fake runner finishing (SC-003); a lost runner after 3 missed pings marks its hosts `lost`;
  logs past `--run-log-cap` keep head and tail with a truncation marker; `SubscribeRun
  { from_seq }` resumes without gaps.
- [ ] T064 [P] [US2] Playwright apps/keel/web/tests/e2e/us2/: a run list, a run page with live
  `LogView`, per-host results table with summary counts, an unreachable host's first failure.

### Implementation

- [ ] T065 [US2] Migration `0004_runs.sql`: `approval_policies` (with the catch-all inserted on
  workspace creation), `runs` (`kind` `normal | rollback | baseline`; `trigger` `delivery | poll |
  manual | api | workflow`; `state`; `state_reason`), `run_hosts` (`state` `queued | sent |
  running | succeeded | failed | refused | lost | cancelled`; `refusal` enum as data-model.md),
  `run_host_results`, `run_events`, `run_log_chunks`, and a minimal `hosts` table (completed in
  T080).
- [ ] T066 [US2] Policy resolution in crates/keel-core/src/policy.rs. Pass T060.
- [X] T067 [US2] Frames and codec in crates/keel-exec/src/frames.rs and codec.rs; the
  `keel_events` parser; the `fake` runner session. Pass T061, T062.
- [X] T068 [US2] The `keel_events` callback plugin in
  apps/keel/runner/ansible/callback/keel_events.py: stdout-neutral, one JSON line per event to the
  fd in `KEEL_EVENT_FD`.
- [ ] T069 [US2] Run creation in apps/keel/server/src/sync.rs and runs.rs: on `synced`, for each
  job whose `paths` match the changed files, create a run, pin `refs/keel/runs/<id>` and the
  inventory revision, resolve hosts (by job profiles and `limit`), and apply policy (`queued` when
  no approval is required).
- [ ] T070 [US2] Queue and dispatch in apps/keel/server/src/queue.rs and dispatch.rs:
  `SELECT … FOR UPDATE SKIP LOCKED`, per-host `Assign`, `Pack` (built by
  crates/keel-exec/src/pack_builder.rs from crates/keel-git/src/pack.rs), `Secrets` (only the
  credentials the job references that are granted to one of the host's profiles; never `git_*`),
  `Cancel`; batched writes of `run_events` and `run_log_chunks`; live forwarding to subscribers.
- [ ] T071 [US2] Run commands: `ListRuns`, `GetRun`, `SubscribeRun`, `UnsubscribeRun`,
  `PreviewRunStart` (manual), `PreviewCancel`, `ListJobs`, `ListPolicies`, `PreviewPolicyUpsert`,
  `PreviewPolicyRemove`, `ExplainPolicy`, and the `RunState` event.
- [ ] T072 [P] [US2] Web pages apps/keel/web/src/features/runs/ (list, run page with `LogView`
  from `@companion/terminal`, per-host table, "a newer inventory revision exists" notice).

**Checkpoint**: US2 scenarios in quickstart.md §2 pass with the fake runner; Playwright `us2`
passes.

---

## Phase 7: User Story 7 — Enrol hosts through the base profile (P1)

**Goal**: real host runners: the SSH listener with two CAs, enrollment, certificates, the KRL, the
two-process runner, revocation.

**Independent test**: enrol a systemd Debian container with a token, run a job on it as
`automation`, revoke it, and confirm it can no longer connect (T133 automates this).

### Tests

- [X] T073 [P] [US7] SSH listener tests in crates/keel-exec/tests/listener.rs: only Ed25519,
  `curve25519-sha256`, `chacha20-poly1305@openssh.com` or `aes256-gcm@openssh.com`; shell, exec,
  pty, env, port forwarding both ways, agent and X11 refused; only the `keel-enroll-v1` and
  `keel-runner-v1` subsystems; an unknown key may open only `keel-enroll-v1`.
- [X] T074 [P] [US7] CA and KRL tests in crates/keel-exec/tests/ca.rs: runner certificates carry
  key id `runner:<id>`, principals `host:<name>` and `profile:<p>` for assigned ∩ allowed
  profiles, a unique serial, `--runner-cert-ttl` validity and no extensions; a revoked serial in
  the KRL is refused; a retiring CA stays trusted until its certificates expire.
- [ ] T075 [P] [US7] Enrollment tests in apps/keel/server/tests/enroll.rs: the token
  `keel-enroll-v1.<secret>.<host CA fingerprint>` is single-use, expires after 15 minutes, is
  stored hashed; a reused token is refused, burned and audited; `inventory_name` must match;
  profiles outside `allowed_profiles` are refused.
- [ ] T076 [P] [US7] Runner client tests in apps/keel/runner/tests/client.rs: a server whose host
  certificate is from another CA, a bare host key, or an expired certificate is refused before
  anything is sent; a token whose fingerprint does not match `host_ca.pub` is refused; an expired
  runner certificate within `--runner-max-offline` may only send `RenewCert`.
- [ ] T077 [P] [US7] Executor isolation tests in apps/keel/runner/tests/executor.rs (no systemd):
  the executor forces `--limit <inventory_name>` and `ansible_connection=local`; refuses
  `profile_not_allowed`; writes secrets at `0600` in the run directory and removes them after
  `Finished`; `secret_vars` are written tagged `!unsafe`, so `{{ lookup('pipe', 'id') }}` stays
  untemplated.
- [ ] T078 [P] [US7] Round-trip tests for `ListHosts`, `GetHost`, `PreviewHostEnroll`,
  `PreviewHostRevoke`, `GetSshCa`, `PreviewSshCaRotate` and `HostState`.
- [ ] T079 [P] [US7] Playwright apps/keel/web/tests/e2e/us7/: enrol a host (token shown once with
  copyable `-e keel_enroll_token=…`), see it come online, revoke it.

### Implementation

- [ ] T080 [US7] Migration `0005_hosts.sql`: complete `hosts` (`inventory_name` unique per
  workspace; `ssh_key_fingerprint` unique; `allowed_profiles text[]`; `require_signed`; `state`
  `online | offline | revoked`), `host_enrollments`, `host_profiles`, `ssh_cas` (`kind`
  `host | runner`; `state` `active | retiring | retired`; sealed private key), `ssh_certs`
  (`serial` unique).
- [ ] T081 [US7] CA, certificate issue and KRL in crates/keel-exec/src/ca.rs and krl.rs; both CAs
  generated at first start and sealed; `keel ssh-ca show` prints fingerprints and the
  `@cert-authority` line. Pass T074.
- [ ] T082 [US7] The embedded SSH server in apps/keel/server/src/runner_listener.rs with the
  `russh` config from crates/keel-exec/src/server_config.rs: host certificate, authentication
  rules of contracts/runner.md, enrollment subsystem, runner subsystem, `Hello` checks (refuse
  `ansible_core` below the minimum with `version_too_old`), `RenewCert`, `Krl`, `Ping`/`Pong`
  every 15 s. Pass T073, T075.
- [ ] T083 [US7] Host commands in apps/keel/server/src/hosts.rs: `PreviewHostEnroll`,
  `PreviewHostRevoke` (adds serials to the KRL, closes the session, a running host becomes
  `lost`), `ListHosts`, `GetHost`, `GetSshCa`, `PreviewSshCaRotate`; all audited with actor
  `host:<id>` for host-originated events.
- [ ] T084 [US7] `keel-runner` (network half) in apps/keel/runner/src/runner/: reads
  `/etc/keel-runner/trust/policy.toml` and `host_ca.pub`, generates its Ed25519 key in
  `/var/lib/keel-runner/id_ed25519` (`0600`) on first start, enrolls from `enroll-token` and
  deletes it, connects, renews at half lifetime, and forwards `Assign`, `Pack`, `Secrets`,
  `Approvals`, `Cancel` unchanged to the executor socket; relays `Started`, `Refused`, `Event`,
  `Log`, `Finished`. It never interprets work. Pass T076.
- [ ] T085 [US7] `keel-executor` in apps/keel/runner/src/executor/: one run per socket
  connection; unpack into its Git store (gix, no hooks, no config from the pack); read the job
  from `.keel/jobs.yaml` in the target tree; check profiles against `allowed_profiles`; check
  out into a private run directory; run `ansible-playbook` from `/opt/keel-runner/venv` with
  `KEEL_EVENT_FD`; `Cancel` sends SIGINT, then the runner stops the unit after 30 s;
  `ReadLedger`. Base and replay checks use the ledger from T097. Pass T077.
- [ ] T086 [US7] Reference systemd units in apps/keel/runner/systemd/ matching
  contracts/host-install.md and research R2: `keel-runner.service` (hardening list of R2),
  `keel-executor.socket` (`SocketUser=root`, `SocketGroup=keel-runner`, `SocketMode=0660`,
  `Accept=yes`), `keel-executor@.service` (`User=automation`, `KillMode=control-group`,
  `PrivateTmp=yes`, `ProtectHome=yes`, `RuntimeMaxSec=6h`).
- [ ] T087 [US7] Hash-pinned `ansible-core` (current stable) in
  apps/keel/runner/ansible/requirements.txt (`uv pip compile --generate-hashes`), and the minimum
  version constant in crates/keel-exec.
- [ ] T088 [P] [US7] Web page apps/keel/web/src/features/hosts/ (list with state, enrol dialog,
  revoke).

**Checkpoint**: US7 targeted tests pass; T133 passes when run by the main session.

---

## Phase 8: User Story 8 — Signed mode with reviewed overrides (P1)

**Goal**: signed mode per binding; unsigned ranges wait for a diff review approved with a passkey
that each host verifies against keys pinned on the host; a hash-chained ledger on each host.

**Independent test**: enable signed mode, push a signed commit (runs), push an unsigned commit
(waits for review), approve with a passkey (runs), then send a forged approval directly to a host
(refused).

### Tests

- [X] T089 [P] [US8] `keel-trust` test vectors in crates/keel-trust/tests/vectors/ with expected
  verdicts, one directory each: signed (OpenPGP), signed (SSH), unsigned tip, unsigned commit
  under a signed merge, unknown key, expired PGP subkey, revoked key, downgrade, re-apply of the
  base, baseline, override for another host, wrong origin, UV missing, expired approval, quorum
  short, replay, broken ledger chain. A generator script in crates/keel-trust/tests/gen/ builds
  the repositories deterministically.
- [X] T090 [P] [US8] Challenge test in crates/keel-trust/src/challenge.rs: the JCS encoding of
  `{v:"keel-override-v1", kind, workspace, binding, run, job, base, target, hosts (sorted,
  deduplicated), expires_at (RFC 3339 UTC, seconds)}` matches fixed bytes and its SHA-256.
- [X] T091 [P] [US8] Ledger tests in crates/keel-trust/src/ledger.rs: `hash = SHA-256(prev ‖
  JCS(line without hash))`, first `prev` 32 zero bytes; the base is the `target` of the latest
  `end` with `succeeded` or `failed`; a `run` with an existing `start` is `replay`; a broken
  chain is `ledger_corrupt`.
- [ ] T092 [P] [US8] Server tests in apps/keel/server/tests/signed.rs: the early check makes a run
  `needs_review` at sync time with failing commits listed; hosts grouped by base into
  `review_groups`; turning signed mode off needs step-up and is audited; a binding in signed mode
  refuses a `managed` inventory.
- [ ] T093 [P] [US8] Hostile-server test in apps/keel/runner/tests/hostile.rs: a test server
  with the right CA sends an unsigned pack with passkey-less approvals, a wrong `base`, an older
  target, approvals for another host and a replayed run id; the executor refuses each with
  `unsigned`, `base_mismatch`, `downgrade`, `bad_override` and `replay` (SC-009).
- [ ] T094 [P] [US8] Round-trip tests for the review commands, `PreviewSignedMode`,
  `PreviewBaseline`, signer commands and the `ReviewRequested` event.
- [ ] T095 [P] [US8] Playwright apps/keel/web/tests/e2e/us8/: a review lists commits with
  signature status, flags `.keel/`, inventory and `keel_runner_*` changes, pages through the diff
  file by file, and approves with a passkey and a required comment.

### Implementation

- [X] T096 [US8] Commit range check in crates/keel-trust/src/commit_check.rs per
  contracts/signed-mode.md: complete object graph or `pack_incomplete`; descendant check or
  `downgrade`; walk every commit in `base..target` including merges; OpenPGP via `pgp` over the
  commit without the signature header (key or subkey in `signers`, signing-capable, not expired
  or revoked at committer time); SSH signatures with namespace `git`; `base = none` always
  `needs_override`. The committer name and email are never used. Pass the commit vectors of T089.
- [X] T097 [US8] Challenge, override verification (via `web_passkeys::verify_assertion`, quorum
  `max(policy.override_quorum, quorum from Assign)`, `now < expires_at <= now +
  max_override_age`, host's `inventory_name` in `hosts`) and the ledger in
  crates/keel-trust/src/. Pass T090, T091 and the remaining vectors.
- [ ] T098 [US8] Executor enforcement in apps/keel/runner/src/executor/: read the base from its
  own ledger, never from `Assign`; `base_mismatch` reporting the host's base; run the check;
  verify overrides; append `start` before and `end` after the run; verify the whole chain at start;
  return `ledger_line` in `Finished`; `keel-executor ledger verify`. Pass T093.
- [ ] T099 [US8] Migration `0006_signed.sql`: `review_groups` (`state` `open | approved | denied |
  expired`), `approvals` (`kind` `approval | override`; unique on (`run_id`,
  `approver_user_id`, `kind`, `review_group_id`); `challenge_sha256?`; `assertion jsonb?`),
  `trusted_signers` (`kind` `openpgp | ssh`; fingerprint unique per workspace), `host_revisions`
  (insert-only).
- [ ] T100 [US8] Early check and review grouping in apps/keel/server/src/review.rs using the same
  `keel-trust` functions against the mirror and workspace signers; diff for review from
  crates/keel-git/src/diff.rs, with trust or targeting flags. Pass T092.
- [ ] T101 [US8] Review commands: `ListReviews`, `GetReview`, `GetReviewDiff` (paged),
  `PreviewOverride` (replies the canonical challenge; `Confirm` carries the passkey assertion over
  it; comment required; four-eyes against unsigned commit authors when the policy asks),
  `PreviewOverrideDeny`, `PreviewBaseline`, `PreviewSignedMode`; signer commands `ListSigners`,
  `PreviewSignerAdd`, `PreviewSignerRemove` (step-up). Approvals are sent to hosts unchanged in
  `Approvals`.
- [ ] T102 [P] [US8] Web page apps/keel/web/src/features/reviews/ (commit list, flagged files,
  paged diff, passkey approve with comment, deny) and a signers section in workspace settings.

**Checkpoint**: `cargo test -p keel-trust` vectors pass on both server and executor paths; the US8
rows in quickstart.md §2 pass; Playwright `us8` passes.

---

## Phase 9: User Story 9 — Host profiles, profile-scoped roles and revision tracking (P2)

**Goal**: profiles assigned within each host's ceiling, profile-scoped grants enforced for hosts,
runs and reviews, and per-host applied revisions with drift.

**Independent test**: assign two hosts different profiles, scope an operator to one, confirm runs
there and refusals elsewhere, and see each host's applied revision and drift.

### Tests

- [ ] T103 [P] [US9] Server tests in apps/keel/server/tests/profiles.rs: `PreviewHostProfiles`
  outside `allowed_profiles` is refused; a push touching one profile's files runs only on that
  profile's hosts; an operator scoped to `gameserver` starting a run on a `debian`-only host is
  `Denied` and audited; approving a review whose hosts span both needs an approver covering all;
  `ReviewRequested` reaches only approvers whose scope covers its hosts.
- [ ] T104 [P] [US9] Revision tests: `Finished.ledger_line` is stored in `host_revisions`; the
  latest row per (`host_id`, `job_id`) is the host's base; drift is the commit count from it to
  `last_commit`; `HostRevision` reaches the host page within 30 s (SC-011).
- [ ] T105 [P] [US9] Playwright apps/keel/web/tests/e2e/us9/: host page with per-job applied
  commit, time, run, signed or overridden, and drift; host list filtered by profile and drift.

### Implementation

- [ ] T106 [US9] `PreviewHostProfiles`, `ListProfiles`, certificate reissue on profile change, and
  profile-aware host resolution in apps/keel/server/src/hosts.rs and runs.rs. Pass T103.
- [ ] T107 [US9] Profile scope checks in every run, review, cancel and approve handler via
  `keel_core::authz` (T018).
- [ ] T108 [US9] `host_revisions` recording, `ListHostRevisions`, drift computation and the
  `HostRevision` event. Pass T104.
- [ ] T109 [P] [US9] Web host page sections and filters in apps/keel/web/src/features/hosts/.

**Checkpoint**: US9 rows of quickstart.md §2 pass; Playwright `us9` passes.

---

## Phase 10: User Story 3 — Govern access, approvals and auditability (P3)

**Goal**: approvals bound to the pinned commit and inventory revision, with TTL, four-eyes and
step-up; searchable, exportable audit.

**Independent test**: require approval on a job; the author's approval is refused; another
approver approves with step-up and the run starts; a later push cannot reuse the approval; audit
holds every action with actor and time.

### Tests

- [ ] T110 [P] [US3] Approval tests in crates/keel-core/src/approval.rs: an approval is valid only
  for its (commit, inventory revision, hosts) tuple; after `queued_at + approval_ttl` it is refused
  and the run moves to `expired`; four-eyes refuses the pusher; `approvals_needed` distinct
  approvers; a role change mid-run does not stop the run.
- [ ] T111 [P] [US3] Server tests in apps/keel/server/tests/approvals.rs for the US3 rows of
  quickstart.md §2, and audit search by repository, host, actor, time range and outcome served
  from the indexes (`EXPLAIN` uses them); `GET /api/v1/audit/export` returns JSON lines for the
  same filter.
- [ ] T112 [P] [US3] Playwright apps/keel/web/tests/e2e/us3/: approve with step-up, see a denied
  approval, filter audit.

### Implementation

- [ ] T113 [US3] Approval logic in crates/keel-core/src/approval.rs. Pass T110.
- [ ] T114 [US3] `PreviewApprove` and `PreviewDeny` with step-up, `ApprovalRequested`, and the TTL
  expiry on read and at dispatch (no timers while idle) in apps/keel/server/src/approvals.rs.
- [ ] T115 [US3] `ListAudit` with cursor paging and `GET /api/v1/audit/export` in
  apps/keel/server/src/audit.rs. Pass T111.
- [ ] T116 [P] [US3] Web pages apps/keel/web/src/features/approvals/ and audit/ (filters for
  workspace, actor, action, binding, host, decision, time).

**Checkpoint**: US3 rows of quickstart.md §2 pass, including `keel audit verify` (SC-004).

---

## Phase 11: User Story 5 — Rollback and migration (P2)

**Goal**: rollback through the same policy, and signed `.keelx` export and import with dry-run
and skip or rename conflicts.

**Independent test**: roll back a failed run; export a workspace and import it into a second
schema with a conflict renamed; a tampered bundle is refused before anything is written.

### Tests

- [ ] T117 [P] [US5] Rollback tests in crates/keel-core/src/rollback.rs: `rollback.playbook` runs
  at the same pinned commit on hosts that changed or failed; `reapply: last_success` uses the last
  success's commit and inventory revision; a missing pin is `artifact_missing` with no fallback;
  no rollback config is `rollback_unavailable`; in signed mode a reapply to an ancestor needs an
  override.
- [ ] T118 [P] [US5] Import planning tests in crates/keel-core/src/migration.rs: every check of
  contracts/export-bundle.md in order (hashes, signature, trusted source key, `format`, closed
  references, conflicts each needing `skip` or `rename:<new>`; no `overwrite`).
- [ ] T119 [P] [US5] Server tests in apps/keel/server/tests/migration.rs: export then import into
  a second schema in one transaction; imported revisions get `imported_from`; no credential
  values, receiver secrets, CA keys, hosts or audit rows cross; a tampered bundle is refused
  before any write (SC-007).
- [ ] T120 [P] [US5] Playwright apps/keel/web/tests/e2e/us5/: roll back a run, export, plan an
  import with a conflict, rename it, apply.

### Implementation

- [ ] T121 [US5] Rollback in crates/keel-core/src/rollback.rs and `PreviewRollback` (step-up,
  `kind: rollback`, `reverses_run_id`) in apps/keel/server/src/runs.rs. Pass T117.
- [ ] T122 [US5] Migration `0007_migration.sql`: `instance` (sealed Ed25519 key),
  `trusted_instances`, `migrations` (`direction` `export | import`; `state` `planned | applied |
  refused`).
- [ ] T123 [US5] Export in apps/keel/server/src/migration.rs: the `.keelx` tar of
  contracts/export-bundle.md, canonical JSON, signed manifest; `PreviewExport` replies a download
  URL valid for 10 minutes.
- [ ] T124 [US5] Import: upload, `PlanImport`, `PreviewImport` (step-up; trusting a new source
  fingerprint needs step-up), one-transaction apply, audited. Pass T118, T119.
- [ ] T125 [P] [US5] Web page apps/keel/web/src/features/migration/.

**Checkpoint**: US5 rows of quickstart.md §2 pass.

---

## Phase 12: Polish and cross-cutting

- [ ] T126 [P] Dockerfile apps/keel/Dockerfile: cargo-chef planner, cook, builder; a separate
  pnpm web stage; target `server` (distroless, non-root, read-only root) with the web `dist`
  copied beside the binary and served from `KEEL_STATIC_DIR`.
- [ ] T127 [P] apps/keel/compose.yaml and .env.example: services `keel` and `keel-db`
  (PostgreSQL 18), both `restart: ${RESTART_POLICY:-no}`; networks `edge` (keel only, web and
  runner ports) and `db` (internal); the master key mounted read-only; named volumes, never
  pruned by tooling.
- [ ] T128 [P] Release build for `keel-runner` and `keel-executor` (static, amd64 and arm64) with
  SHA-256 sums, as a Makefile target `keel-runner-release`, so 058 can pin them in Ansible vars.
- [ ] T129 [P] OpenTelemetry spans for delivery → sync → run → host, and metrics for queue depth
  and run duration, in apps/keel/server/src/telemetry.rs.
- [ ] T130 [P] Guides in docs/guides/keel/: `install.md`, `binding-a-repository.md`,
  `approvals.md`, `host-runners.md` (including the threat model's residual risks and the
  recommendation to use signed mode), `signed-mode.md`, `profiles.md`, `migration.md`. Link them
  from the README; developer setup and architecture go in CONTRIBUTING.md.
- [ ] T131 Idle check: with nothing bound or open, the server sets no timers and stays under 1% of
  a core (a test in apps/keel/server/tests/idle.rs counting scheduled wake-ups).
- [ ] T132 Secret-leak sweep test in apps/keel/server/tests/leaks.rs: after the US1–US9 scenarios,
  no credential value, receiver secret or enrollment token appears in `audit_event`, `run_events`,
  `run_log_chunks`, logs or an export.
- [ ] T133 (main session, opt-in) `KEEL_IT_HOST=1 cargo test -p keel-runner --test host`, per
  quickstart.md §3, in a systemd Debian container named `keel-it-host-<pid>` that the test removes.
- [ ] T134 (main session) quickstart.md §4 end to end on the dev compose with a throwaway VM.
  Compose `down` leaves volumes; ask before removing any.
- [ ] T135 (main session) One full `scripts/ci.sh` on the combined branch, then
  `scripts/hygiene.sh`.
- [ ] T136 (main session, last) Brochure screenshots of the Keel screens.

---

## Dependencies and execution order

### Phases

- Setup → Foundational → US6 → US1 → US4 → US2.
- After US2: **track A** US7 → US8 → US9, and **track B** US3 and US5 (independent of each other).
- US8's executor enforcement (T098) extends the executor from US7 (T085).
- US5's signed-mode reapply rule (in T117) needs `keel-trust` from US8; write that row last or
  mark it pending until US8 lands.
- Polish after the stories it touches. T133, T134, T135 and T136 run in the main session only,
  one at a time, in that order.

### Cross-spec

- **054**: `crates/web-passkeys` is created here (T006, T023). When 054's US4 is built, Depths
  uses the crate, and `cargo test -p depths-server --test passkeys` becomes part of the gate.
- **058**: onboarding `../ansible` (the `keel_runner` role, trust vars, cut-over) starts only
  after this spec is implemented and T128 has produced release binaries.

### Within each story

Tests first (they must fail), then migrations, then core logic, then server handlers, then web.

---

## Parallel execution examples

### After Foundational (`CARGO_BUILD_JOBS=4`, no CI, targeted tests only)

```text
Agent A (trust):    T089–T091 vectors and tests, then T096–T097 (pure crate, no server needed)
Agent B (SSH):      T073–T074, then T081 and the listener half of T082 (crates/keel-exec)
Agent C (web):      T038, T052, T059 pages against the fake backend
Main session:       US6 and US1 server work, migrations (one author for migrations, to keep
                    their numbering linear), merges
```

Migrations (`apps/keel/server/migrations/`) and `crates/keel-protocol` are edited by one agent at a
time; the others leave notes for the main session.

### User Story 1

```text
T039, T040, T041, T042, T043 together (tests)
T045 (keel-git) and T046 (keel-core manifest) together, then T048–T051 in the server
```

### User Story 8

```text
T089, T090, T091, T093, T094, T095 together (tests)
T096–T097 (keel-trust) alongside T099–T100 (server)
```

---

## Implementation strategy

### MVP first

1. Setup, Foundational, US6 and US1: a governed instance that accepts and audits deliveries and
   reads jobs. Nothing executes yet.
2. US4 and US2: runs end to end with the fake runner.
3. US7: real hosts. **Do not point a real host at Keel before US8** unless the host's trust
   policy sets `require_signed = false` knowingly; signed mode is what makes a compromised Keel
   harmless.

### Incremental delivery

1. MVP above.
2. US8 then US9: signed mode and profiles. This is the first deploy candidate for the owner's
   fleet, and 058 starts here. Ask before deploying.
3. US3 and US5 in parallel.
4. Polish, the opt-in host test, one full CI, and screenshots last.

### Notes

- The executor trusts nothing the network half says. A task that seems to need the runner to
  interpret a frame is a design error; check research R2.
- Any change to the override challenge fields changes contracts/signed-mode.md, the T090 fixed
  bytes and the vectors together.
- Keel's early check and the executor must call the same `keel-trust` functions. A second
  implementation of any check is a bug.

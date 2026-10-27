# Research: Keel, a Git-bound Ansible control plane (055)

## Threat model

- **In scope:**
  - Hostile repository content. A playbook, role, lookup or `local_action` runs arbitrary code on
    the machine that executes Ansible. Anyone who can push to a bound branch can write it.
  - Forged or replayed webhook deliveries.
  - A stolen provider account or session cookie.
  - An insider acting outside their team scope.
  - A tenant reading another workspace's credentials or run output.
- **Goals:**
  - Repository code never runs inside the Keel server, and never sees the database, the master
    key, receiver secrets, or another run's credentials (R1, R2).
  - A delivery is trusted only as a hint that a branch moved. Keel decides what changed by
    fetching the branch itself (R5).
  - A repository change can propose a run, but cannot weaken the approval policy that governs it
    (R7).
  - Every decision that changes state commits in the same transaction as its audit record (R10).
- **Out of scope:**
  - The managed nodes themselves, and what a correctly approved playbook does to them.
  - A compromised Keel server. It holds the master key, so it can decrypt credentials. The server
    stays small and never runs repository code, which is what keeps this risk narrow.
- **Residual risk** (stated in the guide):
  - A runner executes whatever an approved job contains, with that job's credentials.
  - A runner pool shared by several workspaces lets one workspace's playbook observe the runner
    while it is idle. Pools are per workspace by default (R2).

## R1. Where Ansible runs

- **Decision:** add a separate service, `keel-runner`, which runs `ansible-playbook` as a
  subprocess. Its image is built from the same Dockerfile and contains `ansible-core`, Python and
  OpenSSH, and nothing from the server.
  - The server never executes Ansible or repository code.
  - Runners connect outbound to the server's runner listener (contracts/runner.md). A runner
    opens no port, so adding one is a matter of starting another container.
  - Each run receives a bundle: the repository tree at the pinned commit, the rendered inventory
    revision, extra vars and the resolved credentials. Credentials are written to a `tmpfs` at
    `0600`.
- **Why:** playbooks are code. AWX isolates them in execution-environment containers. Keel gets
  the same boundary with no Docker socket. Giving Keel a socket would make it root on the host,
  which is the problem spec 054 had to solve for Depths.
- **Alternatives rejected:**
  - Running `ansible-playbook` inside the server. One bad role would then read the master key and
    the database.
  - A container per run (AWX-style). This needs a container runtime socket, which is
    root-equivalent (054).
  - `ansible-runner`. It adds a Python supervisor and artifact directory that Keel would then
    have to parse. The callback in R3 is about 100 lines and gives Keel exactly the events it
    needs.

## R2. Isolation inside a runner

- **Decision:** a runner executes one run at a time.
  - The supervisor (`keel-runner`, uid 10001) starts each run as a separate uid (10002) with
    `setpriv`, in a new session.
  - The container keeps only `CAP_SETUID`, `CAP_SETGID` and `CAP_KILL`. It has a read-only root,
    `no-new-privileges`, a `tmpfs` work directory, and pids and memory limits.
  - After every run, whatever the outcome, the supervisor kills every process owned by uid 10002,
    checks that none remain, and wipes the work directory. If processes survive, the runner
    marks itself `tainted`, refuses further work, and reports this to the server.
  - Runners belong to a **runner pool**. A pool is bound to one workspace by default. An admin
    may share a pool across workspaces, after a preview that names every workspace involved.
- **Why:** a playbook can start a background process to watch the next run. Running one job at a
  time under a separate uid, then killing that uid, closes this off without user namespaces,
  which Docker's default seccomp profile blocks.
- **Alternatives rejected:**
  - Several concurrent runs in one runner. They would share a uid and could read each other's
    `tmpfs`.
  - bubblewrap or user namespaces. These need a relaxed seccomp profile on the host.

## R3. Per-node results

- **Decision:** the runner image ships a stdout-neutral callback plugin, `keel_events`, enabled
  through `ANSIBLE_CALLBACKS_ENABLED`.
  - It writes one JSON line per event to the file descriptor named in `KEEL_EVENT_FD`. The events
    are `playbook_start`, `play_start`, `task_start`, per-host
    `ok/changed/failed/unreachable/skipped`, and the final `stats`.
  - The runner relays these to the server as frames. Human-readable stdout and stderr go
    separately as log chunks.
  - `no_log` results are recorded only as `censored`, which Ansible already enforces in the
    result passed to callbacks.
- **Why:** it is the narrowest interface that gives the per-host outcome table (FR-010) and the
  run summary from Ansible's own `stats`. Parsing human-readable output would be fragile.
- **Version:** use the current stable `ansible-core`, pinned in the runner image, with its Python.
  The image is rebuilt on each upstream minor release. Keel declares the minimum version it
  accepts in the runner's `Hello`.

## R4. Git access

- **Decision:** the server keeps a bare mirror of each bound repository in
  `<data_dir>/mirrors/<binding id>.git`, using `gix` (gitoxide).
  - Supported transports are HTTPS (token) and SSH (deploy key, using the system `ssh` with
    `StrictHostKeyChecking=yes` and a pinned `known_hosts` entry per binding).
  - The tree for a run is exported from the pinned commit into the run bundle. No hooks run and no
    checkout happens on the server.
  - Commits a run used are pinned as `refs/keel/runs/<run id>`, so they survive force-pushes and
    garbage collection. Rollback needs them (R8).
- **Why:** gitoxide is pure Rust, runs no hooks, and needs no Git installation for HTTPS. The
  mirror is a cache: if it is lost, it is refetched.
- **Alternatives rejected:**
  - Shelling out to `git`. This adds a large binary to the server image, plus hooks and config
    behaviour to lock down.
  - `git2` (libgit2). C, with weaker SSH support.

## R5. Receivers

- **Decision:** each receiver has its own URL, `POST /hooks/{receiver id}`, and a provider kind:
  `github`, `gitlab`, `forgejo` (also covers Gitea) or `generic`.
  - **Signature:** HMAC-SHA256 over the raw body (`X-Hub-Signature-256`, `X-Gitea-Signature`,
    `X-Forgejo-Signature`, or `X-Keel-Signature` for generic). GitLab's shared token
    (`X-Gitlab-Token`) is compared in constant time.
  - **Rotation:** two secret slots. Rotating moves the current secret to `previous`, which stays
    valid until an expiry the admin picks (default 24 h). Both slots are checked. This covers the
    edge case where a receiver is rotated while its bindings are active.
  - **Trust:** the payload is used only for the delivery id, the ref name and the repository URL,
    which must match the binding. Keel then fetches the branch and takes the head commit itself.
  - **Duplicates:** deliveries are unique per `(receiver, delivery id)`. A fetch that resolves to
    a commit already processed for the binding records the delivery as `duplicate` and stops.
  - **Out of order:** if the fetched head is an ancestor of the last processed commit, the
    delivery is `stale` and ignored. If history was rewritten, the delivery is refused as
    `history_rewritten` unless the binding allows force-pushes.
  - **Timing:** the decision to accept or refuse is made from the signature and binding checks
    before any fetch, so the response comes back in milliseconds. A delivery that is accepted then
    shows `syncing`, followed by `synced` or `sync_failed`. This meets SC-002 with a wide margin.
  - **Polling:** a binding may poll its branch on an interval as well as, or instead of, using a
    receiver, for repositories that cannot reach Keel.
- **Alternatives rejected:**
  - Trusting the commit in the payload. A replayed or forged payload could pin an old commit.
  - One shared webhook URL. A leaked secret would then affect every binding.

## R6. Job definitions: the repository proposes, Keel disposes

- **Decision:** a bound repository declares its jobs in `.keel/jobs.yaml` (contracts/repo-manifest.md).
  A job names a playbook, an inventory, a target limit, extra vars, credential references by
  name, path filters that trigger it, and an optional rollback (R8).
  - What decides whether a job runs, and who must approve it, lives in Keel: **approval
    policies**, set per workspace and matched by job name, inventory and target pattern.
  - The default policy for a new workspace is "approval required". An admin relaxes it per job.
  - Credential references in the manifest resolve only to credentials the workspace has
    explicitly granted to that binding.
- **Why:** if the repository could mark its own jobs as not sensitive, anyone who can push could
  remove the approval gate (FR-011). Deny by default (FR-016) applies to runs too.
- **Alternatives rejected:**
  - Defining jobs only in the UI, as AWX job templates are. That breaks "the repository is the
    system of record".
  - Defining policies in the repository. See "Why".

## R7. Runs and approvals

- **State machine:** `pending_approval`, `queued`, `running`, then one of `succeeded`, `failed`,
  `cancelled`, `denied` or `expired`. `denied` covers a refused approval and a policy refusal.
- **Pinning:** when a run is created, Keel pins its commit and its inventory revision. If the
  inventory changes between the trigger and execution, the run still uses the pinned revision,
  and the run page shows "a newer inventory revision exists".
- **Approvals:**
  - An approval request names the commit, the diff since the last successful run, the inventory
    revision, and the full list of resolved target hosts.
  - An approval is valid only for that exact pinned tuple. Approvals expire (default 24 h); an
    approval made after expiry is refused and the run moves to `expired`.
  - The approver must hold the approver role in the run's workspace, must not be the person who
    pushed the commit when the policy says "four eyes", and must pass passkey step-up (R11).
- **Disabled bindings:** disabling a binding moves its queued and pending runs to `cancelled`,
  with reason `binding_disabled`. Running runs finish. History is kept (FR-015).
- **Team changes during a run:** a run executes with the authority checked when it was approved
  or queued. A role change mid-run does not stop it, but a cancel by anyone still authorized
  does. Both facts are audited.

## R8. Rollback

Ansible has no built-in undo, so Keel supports the two forms that operators actually use. Each
job opts in through its manifest.

- **`rollback.playbook`:** a named rollback playbook, run against the hosts that changed or
  failed, at the same pinned commit.
- **`rollback.reapply: last_success`:** re-run the job at the commit and inventory revision of
  its last successful run. These are pinned refs (R4) and immutable revisions, so they always
  exist. If the pinned ref is missing because the mirror was rebuilt and the commit is gone
  upstream, the rollback is refused with `artifact_missing`. It never falls back.
- A rollback is itself a run, with `kind: rollback` and a link to the run it reverses. It goes
  through the same approval policy, plus step-up.
- **Inventory rollback:** creates a new revision whose content equals an earlier one. Revisions
  are never edited or deleted.

## R9. Inventory

- **Sources:** an inventory is either `repo` (a path in a bound repository) or `managed` (edited
  in Keel). It is never both.
  - `repo`: YAML or INI inventory files, plus adjacent `group_vars/` and `host_vars/`.
  - Dynamic inventory plugins and scripts are out of scope for v1, because they would run code on
    the server.
- **Revisions:** every sync or edit that changes content makes an immutable revision with a
  content hash, the source commit (for `repo`) or the editor (for `managed`), and a parent.
- **Validation** runs before a revision can be used (FR-020):
  - It parses in Rust (`keel-core::inventory`) to a normalized host, group and vars model.
  - Host and group names follow Ansible's rules. Group nesting has no cycles. There are no
    duplicate host identities.
  - Credential references must resolve to granted credentials.
  - An optional per-inventory JSON Schema applies to host and group vars.
  - A heuristic warns on likely plaintext secrets (keys named `*password*`, `*token*` or
    `*secret*` that hold plain strings). Vault-encrypted values are allowed.
  - A revision that fails validation is stored with its errors and status `invalid`, so it is
    visible, but no run can use it.
- **Deprecation:** a deprecated inventory accepts no new runs and keeps its history.

## R10. Storage and audit

- **Decision:** PostgreSQL (current stable major, 18 at the time of writing), using `sqlx` with
  compile-time checked queries and `sqlx migrate`. This is Keel's own database; nothing is shared
  with other products (FR-025).
  - **Job queue:** a `runs` table with `SELECT … FOR UPDATE SKIP LOCKED`. The server hands
    claimed runs to connected runners.
  - **Audit:** the `audit_event` table is append-only.
    - The application's database role has `INSERT` and `SELECT` on it, but not `UPDATE` or
      `DELETE`.
    - A trigger refuses updates and deletes even for the owner.
    - Each row carries `prev_hash` and `hash` (SHA-256 over the canonical entry and the previous
      hash), so editing a row offline is detectable. `keel audit verify` checks the chain.
  - Every state-changing command writes its audit row in the same transaction as the change, so
    no change exists without its audit row (SC-004).
- **Why:** runs, per-host results, revisions and team grants are relational and need search
  (FR-014). The repository's convention for relational data is PostgreSQL.
- **Alternatives rejected:**
  - SQLite. It would be simpler to deploy, but has weak concurrent writes for streaming run
    events, and goes against the house database choice.
  - The shared `audit-log` crate (JSON-lines files). It is not transactional with the state
    change, and searching it at Keel's volume would mean a second index.
  - NATS JetStream for the queue. FR-025 forbids requiring an event transport in standalone mode.

## R11. Identity, roles, sessions and step-up

- **Sign-in:** the shared `web-auth` crate (OIDC, GitHub, Discord), as Depths uses it. An identity
  is the issuer plus the subject (057).
- **Roles:**
  - Instance roles are `owner`, `admin` and `user`, as in Depths.
  - Team roles inside a workspace are `viewer`, `operator` (runs, cancels) and `approver`.
    Workspace admin is a team role on the workspace itself.
  - Deny by default: an identity with no grant sees nothing.
- **Immediate revocation (FR-017):** every user has an `authz_epoch`. Removing a user, or any
  role or team change that narrows their access, increments it in the same transaction.
  - Sessions and WebSocket connections record the epoch they were authorized at, and check it on
    every command.
  - When the epoch changes, the server closes the user's sockets with `revoked`.
- **Step-up:** passkeys, as in 054. Step-up is required to:
  - approve a run, start a rollback, or import a migration;
  - create, rotate or grant a credential;
  - create or rotate a receiver;
  - change a role.
- **Passkeys:** 054 kept passkeys in the Depths server until another app needed them. Keel does,
  so they move into a shared crate, `crates/web-passkeys` (store, ceremonies, recovery codes).
  Depths switches to it in the same change, and its tests are the regression gate.

## R12. Credentials

- **Kinds:** `ssh_key` (with an optional passphrase), `become_password`, `vault_password` and
  `env` (named variables for cloud collections).
- **At rest:** sealed with XChaCha20-Poly1305 under a per-credential data key, which is wrapped by
  the master key read from `KEEL_MASTER_KEY_FILE`.
  - The key file is host-owned, mode `0400`, and is not in the database.
  - If the file is missing or unreadable, Keel refuses to start. It never generates a key over
    existing data.
- **In use:** credentials are decrypted only when a run's bundle is assembled, sent once over the
  runner channel, written to the runner's `tmpfs` at `0600`, and wiped with the work directory.
  They never appear in audit, run records, logs or the protocol. The protocol carries only the
  name, kind, fingerprint and grants.
- **Suite mode:** a credential may instead be a reference to a Ballast secret
  (`ballast:<namespace>/<name>`), resolved through Ballast's public API with a per-run scoped
  token. This uses the `SecretResolver` port, with a `local` adapter and a `ballast` adapter. It
  is only available when both products are enrolled.

## R13. Migration between Keel instances

- **Export:** a `.keelx` bundle, which is a tar file (contracts/export-bundle.md).
  - It contains a manifest, workspaces, bindings without secrets, approval policies, inventories
    with all their revisions, job-run history (optional), and credential *references* without
    values.
  - Each entry has a SHA-256 hash. The manifest is signed with the source instance's Ed25519
    instance key.
- **Import:**
  1. Verify the signature, and that the source instance key is trusted by the importing admin.
     Trust is on first use, with the fingerprint shown.
  2. Check that no reference is orphaned.
  3. Dry-run to a diff.
  4. Apply.
  - **Conflicts:** a conflict is a workspace slug, an inventory name or a host identity that
    already exists with different content. Conflicts are refused unless the admin picks
    `skip` or `rename` for each one; `overwrite` is not offered (US5 scenario 3).
  - Imported revisions keep their source ids as provenance. The import is audited with both
    instances' ids and the bundle hash (SC-007).
  - Credentials must be re-entered, or re-pointed to Ballast, on the destination.

## R14. Suite mode

Suite mode is built against 057's contract when that contract exists. Keel does not wait for it:
the standalone data model already carries what suite mode needs.

- Every grant has an `origin` (`local` or `suite`). Suite grants are read-only.
- The authorizer takes local grants plus an optional `SuiteBundle` from the `SuitePolicy` port.
  The standalone adapter returns none.
- Audit rows have stable UUIDs, so they can later be forwarded through an outbox table with
  at-least-once delivery.
- The public API is the same protocol over `POST /api/v1/cmd`, with bearer tokens scoped to a
  workspace and to actions (FR-044). Workflow-invoked runs still pass Keel's approval policy and
  step-up (FR-027).
- The scope mapping (FR-028) is a table from suite scope to workspace and project.

When 057 is planned, the bundle verification and outbox code should be a shared crate used by all
four products. Keel's adapter then becomes a thin wrapper around it.

## R15. Web client

- `apps/keel/web` uses React, TypeScript, Vite, `packages/ui`, `packages/theme`,
  `packages/branding` (`舵`, already present) and `packages/terminal`. The terminal package's
  `LogView` provides virtualized run output and per-host filtering.
- It follows the Depths web layout and its fake-backend testing pattern: a scripted protocol
  backend, Playwright projects per story, and Vitest for view logic.

## R16. Operations

- **Image:** a multi-stage Dockerfile with `cargo-chef` (planner, cook, builder), a separate
  `pnpm` web stage, and targets `server` (distroless) and `runner` (Debian slim with
  `ansible-core`).
  - The server serves the web `dist` from `--static-dir` / `KEEL_STATIC_DIR`; assets are never
    embedded in the binary.
- **Compose:** services `keel`, `keel-runner`, `keel-db` (PostgreSQL), all with
  `restart: ${RESTART_POLICY:-no}`. The runner listener is on an internal network only.
- **Telemetry:** OpenTelemetry (traces for trigger to sync to run; metrics for queue depth and run
  duration). It is off unless configured, as in Depths.
- **Runtime flags:** suite enrollment, polling and shared runner pools are runtime settings, not
  build features.

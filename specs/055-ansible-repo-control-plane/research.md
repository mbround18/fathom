# Research: Keel, a Git-bound Ansible control plane (055)

## Threat model

- **In scope:**
  - Hostile repository content. A playbook, role, lookup or `local_action` runs arbitrary code on
    the host that executes it, usually as root. Anyone who can push to a bound branch can write it.
  - A stolen Git account, or a stolen signing key that is not on the trusted list.
  - Forged or replayed webhook deliveries.
  - A stolen provider account or session cookie.
  - An insider acting outside their team or profile scope.
  - A tenant reading another workspace's credentials or run output.
  - **A compromised Keel server** (new with host runners). It holds the master key, so it can read
    credentials and refuse or delay runs. In signed mode it MUST NOT be able to make a host
    execute anything that a trusted signer or a pinned approver did not authorize (R19, R20).
  - A compromised managed host trying to reach other hosts or Keel's secrets for other hosts.
- **Goals:**
  - Repository code never runs inside the Keel server, and never sees the database, the master
    key, receiver secrets, or another host's credentials (R1, R2).
  - The host is the last gate. It verifies signatures, approvals, its own last applied revision,
    and its allowed profiles from trust material pinned on the host, not from what Keel says
    (R19–R21).
  - In signed mode everything that can execute comes from the signed tree: playbooks, the job
    definition, the inventory and its vars. Keel supplies only the run id, the commit range and
    secret values, and secrets are passed so that Ansible never templates them (R19).
  - A delivery is trusted only as a hint that a branch moved. Keel decides what changed by
    fetching the branch itself (R5).
  - A repository change can propose a run, but cannot weaken the approval policy that governs it
    (R7). Changes to host trust material are themselves signed commits, flagged in review (R22).
  - Every decision that changes state commits in the same transaction as its audit record (R10).
- **Out of scope:**
  - What a correctly signed or approved playbook does. A job that uses `become` is root on its
    host; Keel's protection is at the gate, not inside an approved job.
  - Root on a managed host that is already compromised by other means.
  - A compromised Keel server **without** signed mode. Then Keel is trusted for content, as AWX
    is. The guide says so and recommends signed mode for any host that matters.
- **Residual risk** (stated in the guide):
  - A compromised Keel server reads every credential it stores, can see run output, and can
    withhold runs. It cannot forge signatures, approvals or a host's ledger.
  - A trusted signer's key, or an approver's passkey together with its device, is enough to run
    code on every host that trusts it. Quorum (R20) raises that to several people.

## R1. Where Ansible runs: on the managed host

- **Decision:** every managed host runs its own `keel-runner`, installed by the operator's base
  Debian profile (R22). Ansible runs on the host with `connection: local` and `--limit` set to the
  host's own inventory name. There are no runner pools and no runner containers.
  - The runner dials out to Keel over SSH (R17). A host opens no port.
  - Keel sends each host a **per-host bundle**: the job name, the commit range, a Git pack of the
    commits the host lacks, and only the secrets that host's job references (R19).
  - The host keeps its own Git store and a ledger of applied revisions (R19), so it does not need
    to trust Keel's view of what it last ran.
  - The Keel server never executes Ansible or repository code, and holds no SSH key that reaches a
    managed host.
- **Why:**
  - Push-based Ansible needs one machine with SSH keys to the whole fleet. That machine is the
    single most valuable target in the system. With host runners nothing holds fleet-wide keys:
    taking a host gives that host, and taking Keel in signed mode gives no code execution.
  - It matches how `../ansible` already works: `make debian` applies the profile locally as root.
  - Each host can enforce its own policy (signed mode, allowed profiles) independently of Keel.
- **Alternatives rejected:**
  - A runner service with SSH keys to every host (the first draft, R1 in commit f70fa44). One
    malicious playbook or a compromised runner reaches every host.
  - Running `ansible-playbook` inside the server. One bad role reads the master key and the
    database.
  - `ansible-pull`. It fetches straight from Git with no approval gate, no signature check beyond
    `--verify-commit`, no per-host results and no secrets delivery.

## R2. Privilege separation on the host

- **Decision:** two processes, two accounts, joined by a root-owned socket.
  - **`keel-runner.service`** (network side) runs as the system user `keel-runner`: no shell, no
    sudo, no group shared with humans. It holds the host's SSH key and certificate, speaks to
    Keel, and passes bundles to the executor. Hardening: `ProtectSystem=strict`,
    `ProtectHome=yes`, `PrivateTmp=yes`, `NoNewPrivileges=yes`, an empty
    `CapabilityBoundingSet`, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`,
    `SystemCallFilter=@system-service`, `MemoryDenyWriteExecute=yes`, and write access only to
    `/var/lib/keel-runner`.
  - **`keel-executor@.service`** runs as the `automation` user, started per run through
    `keel-executor.socket` (`Accept=yes`, `/run/keel-runner/executor.sock`, `SocketUser=root`,
    `SocketGroup=keel-runner`, mode `0660`). Only the network side can connect, and it cannot
    choose the user, the binary or the arguments.
  - The executor **re-verifies everything itself** (R19, R20): it does not trust the network
    side, which faces the network and is the more likely to be compromised.
  - Ansible escalates with `become` through a sudo rule for the `automation` user only. That rule
    is root-equivalent by nature; the gate is what the executor agrees to run.
  - **Cleanup:** each run is one service instance with `KillMode=control-group`. When the run
    ends, systemd kills every process in its cgroup and removes its `PrivateTmp`, so a playbook
    cannot leave an unprivileged process watching the next run. A playbook that becomes root can
    escape the cgroup; that is inside an approved job (threat model).
  - The executor's state (`/var/lib/keel-executor`: Git store, ledger, work directories) is owned
    by `automation`, mode `0700`, with no group bits, so members of the `automation` group get
    nothing from it.
- **Why:** the network side is the part an attacker can reach, so it holds nothing that runs code
  or escalates. The part that runs code accepts only work that passes verification against files
  it cannot write.
- **Alternatives rejected:**
  - One process as `automation`. A parser bug in the network code would be root through `become`.
  - The old uid-10002 taint model inside a container. systemd cgroups give the same guarantee on
    a host with less code.
  - Running the executor as root. Equivalent once `become` is used, but loses the unprivileged
    default for jobs that do not need it, and the user asked for the `automation` account.

## R3. Per-node results

- **Decision:** the runner package ships a stdout-neutral callback plugin, `keel_events`, enabled
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
- **Version:** the current stable `ansible-core`, installed by the base profile into
  `/opt/keel-runner/venv` with `uv` from a hash-pinned requirements file, and owned by root. The
  executor uses only that interpreter. Keel declares the minimum version it accepts in the
  runner's `Hello`.
- **Collections:** the executor never runs `ansible-galaxy install`. Collections and roles are
  either vendored in the repository, and so covered by its signatures, or installed in the venv
  by the base profile with pinned hashes.

## R4. Git access

- **Decision:** the server keeps a bare mirror of each bound repository in
  `<data_dir>/mirrors/<binding id>.git`, using `gix` (gitoxide).
  - Supported transports are HTTPS (token) and SSH (deploy key, using the system `ssh` with
    `StrictHostKeyChecking=yes` and a pinned `known_hosts` entry per binding).
  - For each host, Keel builds a Git pack of the commits between that host's base and the pinned
    commit (R19). No hooks run and no checkout happens on the server.
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

- **State machine:** `needs_review` (signed mode only, R19), `pending_approval`, `queued`,
  `running`, then one of `succeeded`, `failed`, `cancelled`, `denied`, `expired` or `refused`.
  `denied` covers a refused approval and a policy refusal. `refused` means a host's own
  verification refused the work (R19–R21), with the host's reason.
- **Fan-out:** a run targets hosts; each host has its own state and result, because hosts run
  independently and may have different bases.
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

- **Kinds:**
  - Delivered to hosts: `vault_password`, `env` (named variables for collections), and
    `secret_vars` (named values exposed to playbooks as variables).
  - Server-side only, never sent to a host: `git_https_token` and `git_ssh_key` for fetching a
    binding.
  - `ssh_key` and `become_password` from the first draft are gone: Ansible connects locally, and
    `become` uses the `automation` sudo rule.
- **At rest:** sealed with XChaCha20-Poly1305 under a per-credential data key, which is wrapped by
  the master key read from `KEEL_MASTER_KEY_FILE`.
  - The key file is host-owned, mode `0400`, and is not in the database.
  - If the file is missing or unreadable, Keel refuses to start. It never generates a key over
    existing data.
- **Scoping:** a credential is granted to a binding, and optionally to profiles. A host's bundle
  carries only the credentials its job references and that are granted to one of its profiles.
- **In use:** credentials are decrypted only when a host's bundle is assembled, sent once over
  the SSH channel, and written by the executor into the run's private directory at `0600`.
  - `secret_vars` are written to the extra-vars file tagged `!unsafe`, so Ansible never templates
    them. A compromised Keel cannot smuggle a `{{ lookup('pipe', ...) }}` in through a secret
    value.
  - They never appear in audit, run records, logs or the protocol. The protocol carries only the
    name, kind, fingerprint and grants.
- **Ballast references** come with the suite integration spec (R14), through the `SecretResolver`
  port. 055 ships only the sealed `local` adapter.

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

## R14. Suite mode: deferred to a suite integration spec

- **Decision:** 055 builds standalone Keel only. Enrolling in the suite (FR-026 to FR-028) moves
  to a dedicated integration spec, written once all four apps exist, that joins them through
  057's portal in one pass.
- 055 keeps only the seams that are expensive to add later:
  - every team, member and grant row has an `origin` column, always `local` for now;
  - audit rows have stable UUIDv7 ids, so they can be forwarded later;
  - `keel-core` has `SuitePolicy` and `SecretResolver` ports, and ships only their standalone
    adapters (no suite bundle; sealed credentials only).
- Moved to the integration spec:
  - the `suspensions`, `audit_outbox`, `suite_enrollment` and `scope_mappings` tables;
  - `credentials.external_ref` (Ballast references);
  - the suite commands and events.
- The public API (`POST /api/v1/cmd`, R18) stays in 055, because scripts need it without the
  suite.
- **Why:** suite contracts written against three apps that do not exist yet would be guesses.
  The bundle verification and outbox code should be one shared crate for all four apps, and that
  is easier to design with all four in front of you.
- **Alternatives rejected:** building suite mode in 055 against 057's draft contract. It would be
  rewritten when the other apps reveal what the contract actually needs.

## R15. Web client

- `apps/keel/web` uses React, TypeScript, Vite, `packages/ui`, `packages/theme`,
  `packages/branding` (`舵`, already present) and `packages/terminal`. The terminal package's
  `LogView` provides virtualized run output and per-host filtering.
- It follows the Depths web layout and its fake-backend testing pattern: a scripted protocol
  backend, Playwright projects per story, and Vitest for view logic.

## R16. Operations

- **Image:** a multi-stage Dockerfile with `cargo-chef` (planner, cook, builder), a separate
  `pnpm` web stage, and one `server` target (distroless). There is no runner image.
- **Runner release:** `keel-runner` is a static Linux binary (amd64 and arm64) attached to Keel's
  releases with SHA-256 sums. The base profile installs it with the `github_release` role, pinned
  by checksum in Ansible vars (R22).
  - The server serves the web `dist` from `--static-dir` / `KEEL_STATIC_DIR`; assets are never
    embedded in the binary.
- **Compose:** services `keel` and `keel-db` (PostgreSQL), both with
  `restart: ${RESTART_POLICY:-no}`. The SSH runner listener (R17) is published on its own port;
  hosts reach it over the operator's network (for example Tailscale).
- **Telemetry:** OpenTelemetry (traces for trigger to sync to run; metrics for queue depth and run
  duration). It is off unless configured, as in Depths.
- **Runtime flags:** polling and signed mode are runtime settings, not build features.

## R17. Server ↔ runner channel: SSH with Keel as certificate authority

- **Decision:** the runner channel is SSH. Keel embeds an SSH server (`russh`) on the runner
  listener (default port 7831), and runners connect to it as SSH clients. Keel runs two SSH
  certificate authorities, kept in its data directory: a **host CA** and a **runner CA**.
  - **Server identity:** Keel's listener presents an SSH host certificate signed by the host CA.
    Runners trust the host CA (an `@cert-authority` line), not one host key, so Keel can rotate its
    host key without touching any host.
  - **Enrollment:** the one-time token is `keel-enroll-v1.<secret>.<host CA fingerprint>`. The
    runner pins the CA fingerprint before it sends anything, so there is no trust-on-first-use
    window. Tokens are single-use, expire after 15 minutes, are stored hashed, and name the
    inventory host and initial profiles.
  - **Runner identity:** the host's own Ed25519 key, generated at install in
    `/var/lib/keel-runner/id_ed25519` (`keel-runner`, `0600`). An unknown key may only open the
    `keel-enroll-v1` subsystem and send `Enroll { token }`.
  - **Runner certificates:** after enrollment Keel issues a user certificate from the runner CA:
    key id `runner:<id>`, principals `host:<inventory name>` and `profile:<name>` for each
    assigned profile, a serial, 24-hour validity, and no extensions (no pty, no forwarding, no
    agent). Runners renew over the channel after half the lifetime.
  - **Every later connection:** the runner authenticates with its certificate. Keel checks the CA
    signature, validity, the serial against its key revocation list (KRL), and that the
    certificate's key is the enrolled key of a runner that is not revoked.
  - **Expired certificate:** a host that was offline past expiry may authenticate with its bare
    enrolled key, and then may only send `RenewCert`. After `--runner-max-offline` (default
    30 days) it must enrol again with a new token.
  - **Server side of SSH:** Ed25519 keys only; `curve25519-sha256`; `chacha20-poly1305` or
    AES-GCM. Shell, exec, pty, port forwarding, agent forwarding and X11 are all refused. The only
    subsystems are `keel-enroll-v1` and `keel-runner-v1`, which carry length-prefixed JSON frames
    (contracts/runner.md).
  - **Revocation:** revoking a runner adds its serials to the KRL, closes its session at once, and
    refuses every later connection.
- **Why:**
  - SSH is what the operator already trusts and audits, and hosts already allow outbound SSH.
  - Certificates make a runner's identity short-lived and self-describing: the principals say
    which host and profiles it serves, signed by Keel, and a stolen certificate expires within a
    day. The host CA removes host-key pinning churn.
  - Bundles carry live credentials, so the channel must be confidential on any network.
- **Alternatives rejected:**
  - Mutual TLS with pinned self-signed certificates (the previous R17). Sound, but it reinvents
    identity, rotation and revocation that SSH certificates already define.
  - The system `sshd` with a forced command. It would put Keel's protocol behind the host's main
    SSH daemon and its configuration, and needs inbound connections to every host.
  - Plain SSH keys with no CA. No expiry, and the profiles a runner serves would live only in
    Keel's database.
- **Later:** keeping the host key in a TPM through `tpm2-pkcs11`, so it cannot be copied off the
  host.

## R18. API style: one Rust protocol, not GraphQL

- **Decision:** keep one Rust-defined protocol (`keel-protocol`) with ts-rs bindings. The web
  client uses it over the WebSocket, and scripts use it over `POST /api/v1/cmd`.
- **Why:**
  - Constitution II requires one contract per app, defined once in Rust, with types generated
    from it. ts-rs already gives the web client end-to-end typings from that one definition:
    the guarantee GraphQL codegen provides, without a second schema to keep in sync.
  - Depths and Fathom use the same pattern, so Keel shares their fake-backend tests and tooling.
  - Preview/Confirm and step-up map directly onto commands. In GraphQL they would be mutations
    with side channels.
  - Live run logs are already event streams over the socket. GraphQL subscriptions would add a
    second streaming transport.
- **Alternatives rejected:**
  - GraphQL through `async-graphql` as the main API. It means two contracts, and constitution II
    forbids that.
  - A read-only GraphQL facade for integrators. This can be added later as an adapter over
    `keel-core` if someone needs it. It is not part of 055.

## R19. Signed mode and host revision tracking

- **Decision:** signed mode is a per-binding setting. A host may also force it locally
  (`require_signed` in its trust policy, R22), in which case it refuses unsigned work whatever
  Keel's setting says.
- **What must be signed:** every commit in the range from the host's last applied commit for the
  job (the **base**) to the target commit, including merge commits and every commit a merge brings
  in. A signed merge does not vouch for unsigned parents; they are in the range.
  - OpenPGP signatures (`gpgsig`, verified with the `pgp` crate, rpgp) and SSH signatures
    (namespace `git`, verified with `ssh-key`) both count. sequoia is LGPL, so it is not used.
  - A signature counts only if its key is a **trusted signer**: on the workspace list (Keel's
    early check) and in the host's own `signers` trust file (the enforcing check). A key that is
    expired, revoked or lacks signing capability at the commit time does not count. A commit
    signed by an unknown key is treated as unsigned.
  - The committer name and email are not used. Only the key counts.
- **Range rules:**
  - The target must be a descendant of the base. A target that is the base re-applies it (allowed).
    A target that is an ancestor of the base, or unrelated, is a downgrade or rewrite: it needs an
    override (R20), including for `rollback.reapply`.
  - **First run:** a host with no base for a job needs a **baseline**: an approver picks a commit
    and approves it with a passkey (R20, empty base). The host records it as its base.
- **What the host takes from Keel:** only the run id, the job name, the target commit, a Git pack
  of the missing commits, secret values (R12), and any approvals. The job definition is read from
  `.keel/jobs.yaml` **in the signed tree**; the inventory must be a repository inventory in the
  same tree (managed inventories cannot be used in signed mode); extra vars come from the
  manifest. Keel's own copy of the job is used only for display.
- **Verification on the host (`crates/keel-trust`, used by both sides):**
  1. Unpack the pack into the executor's Git store (`gix`, no hooks, no config from the pack).
  2. Check that the target commit id exists and that its object graph is complete.
  3. Read the base from the host's own ledger, never from the bundle. If Keel assumed a
     different base, refuse with `base_mismatch` and report the host's base.
  4. Walk `base..target`; check each commit's signature against the host's signers.
  5. If any fail, require a valid override (R20) for exactly this base and target.
  6. Check the job's profiles against the host's allowed profiles (R21).
  7. Check out the target tree into the run's private directory and run.
- **Ledger:** `/var/lib/keel-executor/ledger.jsonl`, one hash-chained line per run: run id, job,
  base, target, authorization (`signed` with signer fingerprints, or `override` with approval
  ids), start, end, outcome and the previous line's hash. The executor appends before and after
  each run. A run id that is already in the ledger is refused, so approvals cannot be replayed.
  - The runner reports the ledger head and per-job bases in `Hello`, and the per-run line in
    `Finished`. Keel stores them as `host_revisions` and shows each host's applied commit and its
    drift from the branch head (FR-036).
  - A ledger whose chain does not verify makes the host refuse all work with `ledger_corrupt`
    until an operator inspects it.
- **Keel's early check:** at sync time Keel runs the same verification against its mirror and the
  workspace signers, so a run that will need review is `needs_review` at once instead of failing
  on the host.
- **Why:** signatures on Git commits are the one authorization that neither Keel nor a stolen Git
  account can forge. Checking the whole range on the host, against a base the host recorded
  itself, means Keel cannot hide an unsigned commit by lying about history.
- **Alternatives rejected:**
  - Checking only the tip. One signed commit on top would launder unsigned ones beneath it.
  - Verifying only in Keel. A compromised Keel would then be the whole fleet.
  - `git verify-commit` on the host. It needs `gpg` and a keyring per host and gives no range
    semantics.

## R20. Overrides: diff review approved by a passkey the host verifies

- **Review:** a run that fails the signature check enters `needs_review`. Keel groups target
  hosts by base commit; each group gets one review showing:
  - the commits in the range, with author, committer and signature status (signed by whom,
    unsigned, unknown key, bad signature);
  - the full diff from base to target, file by file, with changes to `.keel/`, inventories and
    `keel_runner_*` variables flagged as **trust or targeting changes**;
  - the job, the resolved hosts and the profiles.
- **Approval:** an approver with rights over every host in the group (R21) enters a comment and
  approves with a passkey. The WebAuthn challenge is the SHA-256 of a canonical, versioned
  encoding of:
  `("keel-override-v1", workspace, binding, run id, job, base, target, sorted host names,
  expires_at)`.
  - `expires_at` defaults to 24 hours. Four-eyes applies: the approver may not be the author of
    any unsigned commit in the range, when the policy asks for it.
  - A **quorum** of distinct approvers may be required, per binding in Keel and per host in its
    trust policy (`override_quorum`). The host's number wins when it is higher.
- **Host verification:** the bundle carries each assertion (credential id, authenticator data,
  client data JSON, signature). The executor:
  1. Rebuilds the canonical challenge from its own values (its base, the target, its own host
     name) and the run id, job and expiry in the bundle.
  2. Finds the credential id among `approvers` in its trust files, pinned through `../ansible`
     (R22).
  3. Checks `type` is `webauthn.get`, the challenge, the origin equals the pinned Keel origin,
     the RP id hash, and that user verification was performed.
  4. Verifies the signature (ES256 or EdDSA) over the authenticator data and client data hash.
  5. Checks `expires_at` against its own clock and that the run id is not in its ledger.
  6. Counts distinct approvers against the quorum.
  - The verify-only function lives in `crates/web-passkeys` (`verify_assertion`, pure, no server
    state) and is reused by `keel-trust`. Sign counters are checked by Keel only.
- **Why:** an approval that only Keel checks is worth no more than Keel's integrity. A passkey
  assertion bound to the exact diff, checked against keys pinned on the host, means only a real
  approver with their device can authorize unsigned code, even if Keel is compromised.
- **Alternatives rejected:**
  - Keel signing the approval with its own key. A compromised Keel signs anything.
  - Approver GPG or SSH signatures over the challenge. Workable, but passkeys are already how
    Keel does step-up, and they need no key handling by approvers.

## R21. Profiles and profile-scoped roles

- **Host profiles:** a profile is a named kind of machine (`debian`, `prox-vm`, `gameserver`,
  `k3s-master`, `k3s-agent`, matching the `../ansible` Makefile targets).
  - The repository declares profiles in `.keel/jobs.yaml` and which jobs apply to each
    (contracts/repo-manifest.md).
  - An admin assigns profiles to hosts in Keel, through Preview/Confirm. The enrollment token sets
    the initial profiles.
  - The host's trust policy lists `allowed_profiles`, pinned by `../ansible`. Keel's assignment
    must be a subset; profiles outside it are refused at enrollment, at certificate issue, and by
    the executor for every job. The certificate carries only the intersection.
  - A push that changes a job's paths creates runs only for hosts with a profile that job applies
    to.
- **Profile-scoped roles:** a team grant may list profiles (`profiles` empty means all). Viewers,
  operators and approvers with a scoped grant see and act on hosts that have one of those
  profiles only. A run or review that touches any host outside the scope needs someone whose
  scope covers all of them.
- **Why:** a gameserver operator should not approve changes to the Proxmox hosts. Host-side
  `allowed_profiles` keeps a compromised Keel from turning a gameserver job onto a hypervisor.

## R22. Integration with the operator's Ansible repository (`../ansible`)

- **Decision:** the base Debian profile installs the runner through a new `keel_runner` role,
  enabled per host by `keel_runner_enabled` (default `false`). The role contract is in
  contracts/host-install.md. In outline it:
  - creates the system users `keel-runner` and `automation` (the account the user asked jobs to
    run as; today `automation` is only a group);
  - installs `keel-runner` and `keel-executor` with `github_release`, checksum-pinned;
  - builds `/opt/keel-runner/venv` with hash-pinned `ansible-core`;
  - writes the trust files under `/etc/keel-runner/trust/` (root-owned, readable by `automation`
    and `keel-runner`, writable by neither) from variables: `keel_runner_server` (Keel host and
    port), `keel_runner_host_ca`, `keel_runner_origin`, `keel_runner_trusted_signers`,
    `keel_runner_approvers`, `keel_runner_require_signed`, `keel_runner_allowed_profiles`,
    `keel_runner_override_quorum`;
  - installs the systemd units and the `automation` sudo rule;
  - enrolls once, if `keel_enroll_token` is passed on the command line. The token never goes in
    Git.
- **Trust changes go through signed commits.** Approver keys and signers live in the repository's
  `group_vars`. Changing them is a commit like any other, so in signed mode it must itself be
  signed or approved by the current approvers, and the review flags it (R20).
- **Prerequisites in `../ansible`, found while planning** (fix before the runner runs as
  `automation`; none are changed by 055):
  1. The `automation` group has a recursive read-write ACL on `/root/ansible`, including `.git`,
     and sudo rules let it run `git -C /root/ansible pull/status/fetch` as root. A group member can
     set `core.fsmonitor` or a hook in `.git/config`, and the next root `git` runs it: root for
     every group member. Fix: run Git as the repository's owner rather than root, or remove write
     on `.git` and pass `-c core.fsmonitor= -c core.hooksPath=/dev/null` with `safe.directory`.
  2. The sudo rule `docker compose -f /root/ansible/composes/* up -d` matches any argument text,
     including `../` and extra flags, so a group member can start any compose file: a privileged
     container is root. Fix: a root-owned wrapper that takes a compose name from a fixed list.
  3. Once Keel applies profiles, the manual `git pull` rules are no longer needed and can go.
- **Why:** the user's Ansible repository already defines the base profile and the `automation`
  group. Putting the runner there keeps host setup in one reviewed place, and pinning trust from
  that repository means a host's trust comes from the operator, not from Keel.


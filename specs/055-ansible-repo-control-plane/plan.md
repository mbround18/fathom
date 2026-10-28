# Implementation Plan: Keel, Ansible control plane from source control

**Branch**: `055-ansible-repo-control-plane` | **Date**: 2026-10-27 | **Spec**: [spec.md](spec.md)

## Summary

Keel runs Ansible from Git: a push to a bound branch becomes a pinned, policy-checked,
optionally approved run, with per-host results and a tamper-evident audit trail. Research R1–R22
holds the decisions; the contracts and data model hold the shapes.

1. **Repository and receivers (US1, US2).**
   - Bare mirrors through `gix`, pinned with `refs/keel/runs/<id>` (R4).
   - Per-receiver `POST /hooks/{id}` for GitHub, GitLab, Forgejo and a generic form, with HMAC and
     two-slot rotation. The payload is only a hint: Keel fetches the head itself (R5,
     contracts/receivers.md).
   - The repository proposes jobs in `.keel/jobs.yaml`; Keel's approval policies decide (R6,
     contracts/repo-manifest.md).
2. **Runs and host runners (US2, US5, US7).**
   - A state machine in `keel-core`. Approval is bound to the pinned commit and inventory
     revision, with TTL, four-eyes and step-up (R7). A run fans out to hosts, each with its own
     state.
   - Ansible runs only on the managed hosts, each against itself with `connection: local`. The
     runner is installed by the base Debian profile in `../ansible` (R1, R22,
     contracts/host-install.md).
   - On the host, an unprivileged network half (`keel-runner`) hands work over a root-owned
     socket to an executor that runs as `keel-exec` and re-verifies everything; systemd kills the
     run's cgroup afterwards (R2).
   - The channel is SSH, with Keel's embedded server acting as host and runner certificate
     authority: pinned CA in the token, 24-hour runner certificates naming host and profiles,
     a KRL, and no shell or forwarding (R17, contracts/runner.md).
   - Rollback runs a rollback playbook or reapplies the last success, through the same policy (R8).
3. **Signed mode (US8).** Optional per binding, forceable per host. Every commit since the
   host's own last applied revision must be signed by a trusted OpenPGP or SSH key, or an
   approver must review the diff and approve it with a passkey that the host verifies against
   keys pinned on the host. The host keeps a hash-chained ledger of what it applied, so a
   compromised Keel cannot launder, downgrade or replay (R19, R20, contracts/signed-mode.md).
4. **Profiles and revisions (US9).** Hosts are assigned profiles within a host-side ceiling;
   grants can be scoped to profiles; Keel shows each host's applied revision and drift (R19,
   R21).
5. **Inventory (US4).** Repository-backed or managed inventories, with immutable revisions,
   validation and diff (R9).
6. **Governance (US3, US6).**
   - The `web-auth` sign-in, roles and team grants at Depths parity.
   - `authz_epoch` revocation, and passkey step-up from a new shared `crates/web-passkeys` (R11).
   - An insert-only, hash-chained `audit_event`, written in the same transaction as the change
     (R10).
   - Credentials sealed with XChaCha20-Poly1305 under a host-owned master key (R12).
7. **Migration (US5).** `.keelx` signed export and import, with a dry-run plan and skip or rename
   conflict choices (R13, contracts/export-bundle.md).
8. **Standalone only.** Suite mode (FR-026 to FR-028) moves to a dedicated suite integration spec
   that joins all four apps once they exist (R14). 055 keeps only the cheap seams: `origin`
   columns, stable audit ids, and the `SuitePolicy` and `SecretResolver` ports with standalone
   adapters. SC-008 holds trivially.
9. **API style.** One Rust protocol with ts-rs typings for the web client and for
   `POST /api/v1/cmd`, rather than GraphQL (R18, constitution II).

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19. Managed hosts get Python and
ansible-core (current stable) in a root-owned venv, used only as a subprocess of the executor.

**Primary Dependencies**:

- The existing workspace: axum and tower, tokio, serde, ts-rs, clap and `web-auth`.
- New:
  - `sqlx` (postgres, runtime-tokio, macros) in the server;
  - `gix` in `keel-git`;
  - `chacha20poly1305`, `ed25519-dalek`, `hmac` and `sha2` (RustCrypto);
  - `russh` (SSH server in Keel, client in the runner) and `ssh-key` (certificates, KRL, SSH
    signatures) for the runner channel (R17);
  - `pgp` (rpgp, MIT/Apache) for OpenPGP commit signatures, chosen over LGPL sequoia (R19);
  - `p256` and `ed25519-dalek` for host-side passkey assertion checks, and `serde_jcs` for the
    canonical challenge (R20);
  - `webauthn-rs`, moved from the Depths server into `crates/web-passkeys`.
- Web: `packages/ui`, `theme`, `branding` and `terminal` (LogView). No new web dependency.

**Storage**: PostgreSQL 18 (data-model.md). Git mirrors and export files go in `KEEL_DATA_DIR`.
The master key is a host-owned file.

**Testing**:

- Pure unit suites in `keel-core`.
- Protocol round trips and binding drift checks.
- gix against local bare repositories.
- Server tests against a throwaway PostgreSQL (`KEEL_TEST_DATABASE_URL`), with a fake runner.
- Playwright projects `us1`–`us6` against the fake backend.
- `keel-trust` test vectors shared by server and executor (contracts/signed-mode.md).
- Opt-in real host tests in a systemd container (`KEEL_IT_HOST=1`). See quickstart.md.

**Target Platform**: the server and database on a Linux container host through compose; host
runners on Debian managed hosts (current stable and oldstable, amd64 and arm64) under systemd.
The web client targets the same browsers as Depths.

**Project Type**: a web service with a web UI, plus a host agent.

**Performance Goals**:

- A webhook decision is visible in under 15 s at p99 (SC-002).
- Per-host results appear within 30 s of a run ending (SC-003).
- A live log reaches the browser within 1 s of the runner sending it.
- An idle server uses under 1% of a core, and there is no polling while nothing is bound or open.

**Constraints**:

- Fail closed everywhere: an unknown receiver, an unverifiable signature, a missing credential or
  a lost runner never results in a run.
- Secrets never appear in logs, events, audit or exports.
- Database migrations are additive only.
- Compose services use `restart: ${RESTART_POLICY:-no}`.
- Telemetry is OpenTelemetry, off unless configured. Feature flags are runtime settings.

**Scale/Scope**:

- Tens of workspaces and hundreds of bindings.
- Inventories of up to about 5,000 hosts.
- Up to a few hundred host runners, each with one SSH session.
- Run logs are capped at 64 MiB per run.

## Constitution Check

*GATE: before Phase 0, and again after Phase 1.*

| Principle | Check | Status |
|---|---|---|
| Scope | Constitution 1.4.0 covers Keel, its server tier, the Keel host runner tier on Debian managed hosts, and ansible-core on managed hosts only. | PASS |
| I. Explicit mutation | Every change is a `Preview*` followed by `Confirm`, naming the affected hosts, bindings and users. Runs show the resolved host list and the pinned commit before approval. Refusals reach the user verbatim, as code plus message. | PASS |
| II. One contract | `keel-protocol` is defined in Rust with ts-rs bindings. The public API is the same `CmdEnvelope` over `POST /api/v1/cmd`, not a second API; this is a deliberate choice over GraphQL. Receivers and runners are separate, documented trust-boundary contracts. The domain sits in `keel-core` with ports, and gix and sqlx are adapters (sqlx used directly, per the database exception). | PASS |
| III. Boundary tests | Round trips for every variant. Receiver signature and refusal tables. A state-machine suite. An audit-chain verify. Playwright us1–us6. Opt-in real Ansible. Each behaviour has a failing-first test. | PASS |
| IV. Density | Run lists, deliveries and audit follow the Depths list patterns, mobile-first. The run view reuses `LogView`. | PASS |
| V. Fast and quiet | Push-driven; polling happens only for bindings that ask for it, at their interval. No timers while idle. | PASS |
| VI. User data | Audit is insert-only. Imports never overwrite. Runs pin inventory revisions, which are never edited in place. Volumes are never pruned by tooling. | PASS |
| Platforms | A Linux container host for the server and Debian hosts with systemd for runners, per the 1.4.0 Keel tiers. No desktop tier is affected. | PASS |

Re-check after Phase 1: no new violations. Hosts hold no database access and only their own
secrets; the host ledger is append-only and hash-chained (VI). One protocol serves both the UI and
the API (II); the runner channel and the host install are separate, documented trust-boundary
contracts.

## Project Structure

### Documentation (this feature)

```text
specs/055-ansible-repo-control-plane/
├── plan.md  research.md  data-model.md  quickstart.md
└── contracts/
    ├── protocol.md       # Cmd/Event, Preview/Confirm, public API
    ├── receivers.md      # POST /hooks/{id}: signatures, checks, decisions
    ├── runner.md         # SSH channel, enrollment, frames, local executor socket
    ├── signed-mode.md    # commit check, override challenge and host verification, ledger
    ├── host-install.md   # the keel_runner role contract for ../ansible
    ├── repo-manifest.md  # .keel/jobs.yaml
    ├── export-bundle.md  # .keelx layout, signature, import checks
    └── config.md         # server and host runner flags, compose
```

### Source Code

```text
crates/keel-core/            # agnostic, no I/O
  src/policy.rs  run.rs  approval.rs  rollback.rs  inventory.rs  manifest.rs  migration.rs
  src/ports.rs               # GitSource, SecretResolver, SuitePolicy, Clock
crates/keel-protocol/        # Cmd/Event/types, ts-rs export test
crates/keel-git/             # GitSource port + gix adapter (mirrors, pins, force-push detection)
crates/keel-exec/            # runner frames, SSH server/client config (russh), CA and KRL, pack builder, keel_events parser
crates/keel-trust/           # pure: commit signature range check, override challenge + verification, ledger chain
  tests/vectors/
crates/web-passkeys/         # extracted from apps/depths/server/src/passkeys.rs; adds pure verify_assertion
apps/keel/server/            # binary `keel`: axum, sqlx, receivers, queue, audit, ws + /api/v1/cmd
  migrations/                # sqlx, additive only
apps/keel/runner/            # binaries `keel-runner` (network half) and `keel-executor` (runs Ansible as keel-exec)
  ansible/callback/keel_events.py
  ansible/requirements.txt   # hash-pinned ansible-core for the host venv
  systemd/                   # reference units matching contracts/host-install.md
apps/keel/web/               # React + Vite; features/{bindings,receivers,inventory,runs,approvals,teams,audit,migration}
  tests/e2e/us1..us6/
apps/keel/Dockerfile         # cargo-chef; targets: server (distroless), web; runner ships as release binaries
apps/keel/compose.yaml  .env.example
apps/depths/server/          # uses crates/web-passkeys (no behaviour change)
docs/guides/keel/            # install, binding a repository, approvals, host runners, signed mode, profiles, migration
```

**Structure Decision**:

- The domain lives in `crates/keel-core` with ports for Git, secrets, suite policy and time.
  `keel-git` and `keel-exec` are reusable adapters. `keel-trust` is pure and shared by the server
  and the executor, so both reach the same verdict. `apps/keel/server` and `apps/keel/runner` are
  thin binaries that stitch them together.
- The `keel_runner` Ansible role lives in `../ansible`, the operator's repository. This
  repository ships the binaries, reference units and the role contract only.
- The server uses sqlx directly, with no repository trait, per the database exception.
- The web app follows the Depths layout and shares `packages/*`.
- Passkeys move into `crates/web-passkeys` so that Keel and Depths share one audited
  implementation. Depths' behaviour is unchanged, and its passkey tests are the guard for that.

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| PostgreSQL (new stack component) | Multi-user queue (`SKIP LOCKED`), insert-only audit enforced by grants and a trigger, transactional audit with each change | Files or SQLite: no role-level insert-only enforcement, and a weak concurrent queue for several runners. |
| Host runners with a two-process split | Nothing holds SSH keys to the fleet; each host enforces its own policy; the network-facing half cannot run code or escalate (R1, R2). | A central runner with fleet SSH keys: one compromise reaches every host. One process on the host: a parser bug becomes root through `become`. |
| Python and ansible-core on managed hosts | Ansible is the product's engine, and runs locally on each host | Reimplementing Ansible is out of scope. It lives in a root-owned, hash-pinned venv used only by the executor. |
| SSH with two Keel CAs | Short-lived, self-describing runner identities; host CA rotation without touching hosts; revocation by KRL (R17). | Pinned mutual TLS: sound, but reinvents identity, rotation and revocation. The host's sshd: inbound access to every host. |
| Signed mode with host-verified passkey overrides and a host ledger | A compromised Keel or Git account must not be able to run code on hosts (R19, R20). | Checking only in Keel: Keel becomes the whole fleet. Tip-only checks: laundering under a signed merge. |
| `pgp` (rpgp) and `russh` | Pure Rust, permissive licenses, no `gpg` or `ssh` binaries to drive | sequoia (LGPL); shelling out to `gpg`/`ssh` with keyrings per host. |
| `crates/web-passkeys` extraction (touches Depths) | Two copies of a WebAuthn relying party would drift, and drift there is a security bug | Copying Depths' module into Keel. |
| `gix` | Bare mirrors, pins and ancestry checks without shelling out to `git` with credentials on the command line | The `git` CLI: credential handling through argv and environment, plus parsing its output. |
| Ed25519 instance key | Signed exports (R13). | Unsigned bundles: an import could be tampered with in transit. |

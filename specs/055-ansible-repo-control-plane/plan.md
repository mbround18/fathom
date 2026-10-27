# Implementation Plan: Keel, Ansible control plane from source control

**Branch**: `055-ansible-repo-control-plane` | **Date**: 2026-10-27 | **Spec**: [spec.md](spec.md)

## Summary

Keel runs Ansible from Git: a push to a bound branch becomes a pinned, policy-checked,
optionally approved run, with per-host results and a tamper-evident audit trail. Research R1–R16
holds the decisions; the contracts and data model hold the shapes.

1. **Repository and receivers (US1, US2).**
   - Bare mirrors through `gix`, pinned with `refs/keel/runs/<id>` (R4).
   - Per-receiver `POST /hooks/{id}` for GitHub, GitLab, Forgejo and a generic form, with HMAC and
     two-slot rotation. The payload is only a hint: Keel fetches the head itself (R5,
     contracts/receivers.md).
   - The repository proposes jobs in `.keel/jobs.yaml`; Keel's approval policies decide (R6,
     contracts/repo-manifest.md).
2. **Runs and runners (US2, US5).**
   - A state machine in `keel-core`. Approval is bound to the pinned commit and inventory
     revision, with TTL, four-eyes and step-up (R7).
   - Ansible runs only in a separate `keel-runner` service. It has no database and no master key,
     dials out to the server, runs one job at a time under its own uid, and taints itself if
     processes survive (R1–R3, contracts/runner.md).
   - Rollback runs a rollback playbook or reapplies the last success, through the same policy (R8).
3. **Inventory (US4).** Repository-backed or managed inventories, with immutable revisions,
   validation and diff (R9).
4. **Governance (US3, US6).**
   - The `web-auth` sign-in, roles and team grants at Depths parity.
   - `authz_epoch` revocation, and passkey step-up from a new shared `crates/web-passkeys` (R11).
   - An insert-only, hash-chained `audit_event`, written in the same transaction as the change
     (R10).
   - Credentials sealed with XChaCha20-Poly1305 under a host-owned master key (R12).
5. **Migration (US5).** `.keelx` signed export and import, with a dry-run plan and skip or rename
   conflict choices (R13, contracts/export-bundle.md).
6. **Standalone first.** Suite mode (057 portal, 056 Ballast secrets) plugs in through the
   `SuitePolicy` and `SecretResolver` ports and is off by default (R12, R14). SC-008 holds with
   no suite settings at all.

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19. The runner image adds Python
and ansible-core (current stable), used only as a subprocess.

**Primary Dependencies**:

- The existing workspace: axum and tower, tokio, serde, ts-rs, clap and `web-auth`.
- New:
  - `sqlx` (postgres, runtime-tokio, macros) in the server;
  - `gix` in `keel-git`;
  - `chacha20poly1305`, `ed25519-dalek`, `hmac` and `sha2` (RustCrypto);
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
- Opt-in real-Ansible tests (`KEEL_IT_ANSIBLE=1`). See quickstart.md.

**Target Platform**: a Linux container host (server, runner and database through compose). The
web client targets the same browsers as Depths.

**Project Type**: a web service with a web UI, plus a runner service.

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
- A handful of runners.
- Run logs are capped at 64 MiB per run.

## Constitution Check

*GATE: before Phase 0, and again after Phase 1.*

| Principle | Check | Status |
|---|---|---|
| Scope | Constitution 1.3.0 adds Keel to the scope, the "Keel server (Linux container)" platform tier, and PostgreSQL/sqlx and ansible-core (runner image only) to the stack. | PASS |
| I. Explicit mutation | Every change is a `Preview*` followed by `Confirm`, naming the affected hosts, bindings and users. Runs show the resolved host list and the pinned commit before approval. Refusals reach the user verbatim, as code plus message. | PASS |
| II. One contract | `keel-protocol` is defined in Rust with ts-rs bindings. The public API is the same `CmdEnvelope` over `POST /api/v1/cmd`, not a second API; this is a deliberate choice over GraphQL. Receivers and runners are separate, documented trust-boundary contracts. The domain sits in `keel-core` with ports, and gix and sqlx are adapters (sqlx used directly, per the database exception). | PASS |
| III. Boundary tests | Round trips for every variant. Receiver signature and refusal tables. A state-machine suite. An audit-chain verify. Playwright us1–us6. Opt-in real Ansible. Each behaviour has a failing-first test. | PASS |
| IV. Density | Run lists, deliveries and audit follow the Depths list patterns, mobile-first. The run view reuses `LogView`. | PASS |
| V. Fast and quiet | Push-driven; polling happens only for bindings that ask for it, at their interval. No timers while idle. | PASS |
| VI. User data | Audit is insert-only. Imports never overwrite. Runs pin inventory revisions, which are never edited in place. Volumes are never pruned by tooling. | PASS |
| Platforms | A Linux container host, per the 1.3.0 Keel tier. No desktop tier is affected. | PASS |

Re-check after Phase 1: no new violations. The runner contract keeps secrets out of the server's
logs and keeps the runner out of the database (VI). One protocol serves both the UI and the API
(II).

## Project Structure

### Documentation (this feature)

```text
specs/055-ansible-repo-control-plane/
├── plan.md  research.md  data-model.md  quickstart.md
└── contracts/
    ├── protocol.md       # Cmd/Event, Preview/Confirm, public API
    ├── receivers.md      # POST /hooks/{id}: signatures, checks, decisions
    ├── runner.md         # server ↔ runner frames, enrollment, taint
    ├── repo-manifest.md  # .keel/jobs.yaml
    ├── export-bundle.md  # .keelx layout, signature, import checks
    └── config.md         # server/runner flags and env, compose
```

### Source Code

```text
crates/keel-core/            # agnostic, no I/O
  src/policy.rs  run.rs  approval.rs  rollback.rs  inventory.rs  manifest.rs  migration.rs
  src/ports.rs               # GitSource, SecretResolver, SuitePolicy, Clock
crates/keel-protocol/        # Cmd/Event/types, ts-rs export test
crates/keel-git/             # GitSource port + gix adapter (mirrors, pins, force-push detection)
crates/keel-exec/            # runner frames, bundle builder/reader, keel_events parser
crates/web-passkeys/         # extracted from apps/depths/server/src/passkeys.rs
apps/keel/server/            # binary `keel`: axum, sqlx, receivers, queue, audit, ws + /api/v1/cmd
  migrations/                # sqlx, additive only
apps/keel/runner/            # binary `keel-runner`: connection, setpriv/uid isolation, process sweep
  ansible/callback/keel_events.py
apps/keel/web/               # React + Vite; features/{bindings,receivers,inventory,runs,approvals,teams,audit,migration}
  tests/e2e/us1..us6/
apps/keel/Dockerfile         # cargo-chef; targets: server (distroless), runner (Debian slim + ansible-core), web
apps/keel/compose.yaml  .env.example
apps/depths/server/          # uses crates/web-passkeys (no behaviour change)
docs/guides/keel/            # install, binding a repository, approvals, runners, migration
```

**Structure Decision**:

- The domain lives in `crates/keel-core` with ports for Git, secrets, suite policy and time.
  `keel-git` and `keel-exec` are reusable adapters. `apps/keel/server` and `apps/keel/runner` are
  thin binaries that stitch them together.
- The server uses sqlx directly, with no repository trait, per the database exception.
- The web app follows the Depths layout and shares `packages/*`.
- Passkeys move into `crates/web-passkeys` so that Keel and Depths share one audited
  implementation. Depths' behaviour is unchanged, and its passkey tests are the guard for that.

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| PostgreSQL (new stack component) | Multi-user queue (`SKIP LOCKED`), insert-only audit enforced by grants and a trigger, transactional audit with each change | Files or SQLite: no role-level insert-only enforcement, and a weak concurrent queue for several runners. |
| A separate runner service | Playbooks are arbitrary code. Running them away from the database, the master key and the receiver secrets contains a malicious playbook. | Running in the server process: one bad playbook reads every credential. |
| Python and ansible-core in the runner image | Ansible is the product's engine | Reimplementing Ansible is out of scope; it stays a subprocess in its own image only. |
| `crates/web-passkeys` extraction (touches Depths) | Two copies of a WebAuthn relying party would drift, and drift there is a security bug | Copying Depths' module into Keel. |
| `gix` | Bare mirrors, pins and ancestry checks without shelling out to `git` with credentials on the command line | The `git` CLI: credential handling through argv and environment, plus parsing its output. |
| Ed25519 instance and runner keys | Signed exports and runner enrollment without a CA | mTLS with a private CA: more moving parts for the same pinning. |

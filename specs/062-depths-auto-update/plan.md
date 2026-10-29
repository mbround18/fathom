# Implementation Plan: Depths image updates across many stacks (Watchtower-compatible)

**Branch**: `062-depths-auto-update` | **Date**: 2026-10-28 | **Spec**: [spec.md](spec.md)

## Summary

Depths learns which services have newer images, shows them on one page, updates many stacks in one
reviewed step, and, for stacks an administrator opts in, updates them unattended in a maintenance
window with a health check and rollback. It reads Watchtower's labels so stacks labelled for
Watchtower behave the same. Research R1–R10 holds the decisions; the contracts and data model hold
the shapes.

1. **Check (US1).** The shared `oci-registry` crate (also used by Keel, spec 061) HEADs each
   service's tag and Depths compares the digest with the running image's repo digests. Unchanged
   tags cost one HEAD (plus a token request) and nothing else; HEAD never falls back to GET. A new
   digest is resolved once (index → host platform manifest → config) and cached by digest, which
   gives the host-architecture comparison, the version labels and the image's user (R1). Checks are
   off until an admin turns them on, spread across the interval, and back off per registry.
2. **Update (US2).** A new per-stack plan: `docker pull <repo>@<digest>`, a hold check on the
   pulled image, `docker image tag`, `docker compose up -d --no-deps --pull never <services>`, a
   health wait, then optional cleanup (R2). It runs through the existing `Ops` lock, preview
   binding and audit log, so batches are one preview and one confirm, then each stack one at a time.
3. **Hold (US2, FR-010).** Before anything is recreated, a service that would go from non-root to
   root, or that the 054 guard would refuse, is held. The local tag has not moved, so a held image
   is never one manual Up away from running (R4).
4. **Unattended (US3).** One scheduler task sleeps until the next check slot or window opening.
   Windows are host local wall time with explicit DST rules and a fake clock and zone for tests
   (R5). Rollback retags the previous image, kept under a `depths-keep/…` tag, and recreates onto
   it; the digest is then rejected (R3).
5. **Watchtower (US4).** A pure label function, table-tested against the vectors in
   contracts/watchtower-labels.md. Running Watchtower containers are found in the existing runtime
   snapshot and pause Auto until acknowledged (R6).
6. **Cleanup (US5).** Only images Depths recorded as a service's previous image, removed without
   `-f` so the daemon refuses anything in use (R7).

## Technical Context

**Language/Version**: Rust 2024 (stable), TypeScript 5 / React 19.

**Primary Dependencies**:

- Existing: tokio, serde, serde_json, chrono 0.4 (`clock` in the server; core takes `now`),
  secrecy, sha2, uuid, opentelemetry, the `docker` CLI and compose plugin already in the image.
- **New shared crate, `crates/oci-registry`** (shared with spec 061; whichever is built first
  creates it). A HEAD-first OCI distribution client with no Keel or Depths types, written on
  dependencies already in the lock and already compiled into depths-server through `web-auth`:
  `reqwest = { version = "0.13.5", features = ["json", "default-tls"] }`, `secrecy = "0.10"`,
  `thiserror = "2"`, `serde`, `serde_json = "1.0.154"`, `sha2 = "0.11"`, `tokio` (`time`),
  `tracing`; optional `axum = "0.8"` behind its `test-util` feature (the fake registry and
  `Client::with_base_url`), enabled only in dev-dependencies. The contract is 061's
  contracts/registry.md. The workspace has no
  `[workspace.dependencies]` table, so the crate pins the same versions the other crates use.
  **No new dependency edges**: no `oci-client`, no `oci-spec`.
- Not added: no cron or scheduler crate (own loop on `tokio::time`), no timezone crate
  (`chrono::Local` reading `/etc/localtime`), no `bollard`, no credential-helper crate.
- No new web dependency.

**Storage**:

- `<data_dir>/updates.json`: settings, policies, rejected digests, Watchtower acknowledgements.
  Admin decisions; unreadable → checks off, Auto paused, file left alone.
- `<data_dir>/update-state.json`: check results, digest resolutions, backoff, run journal.
  Observations; unreadable → moved aside and rebuilt.
- Both written atomically in the `users.rs` pattern. Local `depths-keep/<stack>` image tags on the
  Docker host for the way back (R7, FR-028), visible in `docker images` and documented.

**Testing**:

- `depths-core::updates` unit suites, all deterministic: reference parsing table, label vectors
  (contracts/watchtower-labels.md, 38 cases), effective policy, window evaluation with a fake clock
  and a fake DST zone (spring-forward and fall-back days, windows crossing midnight, clock jumps),
  check/resolve logic over a fake `Registry` port that counts `head_digest` and `resolve` calls (the
  SC-007 budget table in contracts/registry.md), state transitions, keep-tag naming.
- `oci-registry` tests against its own `FakeRegistry` (feature `test-util`: an axum router on
  `127.0.0.1:0`) reached through `Client::with_base_url`: reference parsing, the token flow
  (anonymous and basic), HEAD-only `head_digest` (a missing `Docker-Content-Digest` is
  `NoDigestHeader`, and the request log shows no GET), `resolve` (index, single manifest, no
  platform match, attestation entries skipped, digest mismatch), `RequestCounts` on every result, and no
  secret in `Debug` or captured logs. No real network.
- Server: batch runner and scheduler against the existing `Mock` `ContainerRuntime` and `Fx`
  fixture in `ops.rs`, plus a scripted health sequence (healthy, unhealthy, restart loop, missing
  replica) and the fake port; restart-mid-run → interrupted; corrupt state files; credential
  file parsing (helpers and identity tokens refused, nothing leaked into logs: a test captures the
  tracing output); the port adapter's error mapping.
- Protocol round trips and binding drift checks for every new variant.
- Playwright, one project per user story against the scripted fake backend: `us9-update-check`,
  `us10-update-batch`, `us11-update-auto`, `us12-watchtower`, `us13-image-cleanup`. (054 takes
  `us7-passkeys` and `us8-guard`; renumber at implementation if a later spec took these.)
- Opt-in Docker integration (`DEPTHS_IT_DOCKER=1`): a local `distribution` registry, a stack that
  updates, one whose new image fails its healthcheck (rollback), one that turns root (hold),
  classic and containerd image stores, compose create-before-stop confirmed.

**Target Platform**: Linux Docker host, Docker Engine with compose v2 (as 050). Both image stores.

**Project Type**: web service (Depths server and web UI).

**Performance Goals**:

- A check of 30 stacks with no moved tags finishes in under 30 s and makes 30 HEAD requests.
- Idle cost with checks on: one sleeping task, no polling between slots (Principle V).
- Updates page renders 300 services in under 200 ms on a mid-range phone (virtualised list).

**Constraints**:

- Zero registry requests with default settings (FR-001, SC-001).
- Compose files and labels never written (FR-009, FR-022). Depths' own container never updated.
- Credentials never logged, audited, sent or stored (R1).
- One operation per stack; one unattended stack at a time.
- Compose services keep `restart: ${RESTART_POLICY:-no}`.

**Scale/Scope**: up to ~50 stacks and ~300 services per host; a few registries; one Depths per host.

## Constitution Check

*GATE: before Phase 0, and again after Phase 1.*

| Principle | Check | Status |
|---|---|---|
| I. Explicit mutation | Every update is a preview naming stacks, services, old and new image and the exact argv, then `Confirm`. Policy and settings changes are previews with before/after and a passkey. Unattended runs are the one exception the spec asks for; they are bounded by an admin-confirmed policy, a window, the holds and rollback, and every run is audited under "Depths (scheduled)" with the policy's author. Nothing shows as updated until the runtime and the health wait answer. | PASS (justified exception, see Complexity) |
| II. One contract | All commands, events and shapes in `depths-protocol` with ts-rs bindings and a version bump. The registry is a port in `depths-core`; depths-server implements it over the shared `oci-registry` adapter crate, which holds no app types. | PASS |
| III. Boundary tests | Round trips for every variant; Playwright us9–us13 against the fake backend; label vectors, DST and request-budget suites; each behaviour change failing-first. Existing us1–us8 keep passing. | PASS |
| IV. Density | Updates is one dense list, filterable, mobile-first, rows expandable for labels and reasons. Users see states inline on the stack page. The batch preview reuses the confirm dialog. | PASS |
| V. Fast and quiet | Off by default. When on: a sleeping scheduler, spread checks, HEAD-only for unchanged tags, health polling only during a wait, Watchtower detection from the existing snapshot. | PASS |
| VI. User data | Compose files untouched; Dockge layout untouched; policies in Depths' data dir. Unreadable `updates.json` is never rewritten. Images are removed only when cleanup is on and only those Depths recorded, without force. Audit log append-only. Docs say plainly that rollback does not undo migrations. | PASS |
| Platforms | Linux container host, as before. Socket proxy needs no new permission. | PASS |

Re-check after Phase 1: no new violations. The design added keep tags on the host (visible,
documented, in a `depths-keep/` namespace) and one 054 allowlist line (`POST /images/{name}/tag`);
neither weakens a principle.

## Project Structure

### Documentation (this feature)

```text
specs/062-depths-auto-update/
├── spec.md  plan.md  research.md  data-model.md  quickstart.md
└── contracts/
    ├── protocol.md           # Cmd/Event changes, gates, 054 allowlist amendment
    ├── config.md             # flags, env, compose mounts
    ├── registry.md           # oci-registry crate contract, Registry port, credentials, request budget
    └── watchtower-labels.md  # label semantics, combination rules, 38 test vectors
```

### Source Code

```text
crates/depths-core/src/updates/            # agnostic: no I/O, `now` and zone passed in
  mod.rs
  reference.rs                             # ImageRef parse + Docker normalisation
  labels.rs                                # Watchtower labels -> EffectivePolicy (contract vectors)
  policy.rs                                # stack policy x labels x settings
  window.rs                                # Window, LocalZone trait, next occurrence, DST rules
  registry.rs                              # Registry port, Resolution, Platform, RegistryError
  check.rs                                 # HEAD/compare/resolve; store detection; moved-back
  hold.rs                                  # root escalation (security::is_root_user), lint re-run
  keep.rs                                  # depths-keep tag names
crates/depths-core/src/plan.rs             # Operation::ImageUpdate / ImageRollback steps
crates/depths-core/src/ports.rs            # ContainerRuntime: image_inspect, containers_health, host_platform (default Unsupported)
crates/depths-core/src/role.rs             # CommandKind::{ViewUpdates, CheckUpdates, ManageUpdates}
crates/oci-registry/                       # NEW shared adapter crate (061 + 062): HEAD-first client on reqwest
  src/{lib,reference,auth,client,manifest,error,testing}.rs   # contract: 061 contracts/registry.md
  tests/client.rs                          # against testing::FakeRegistry (feature test-util), request log
crates/depths-runtime-cli/src/lib.rs       # implement the three new port methods
crates/depths-protocol/src/{types,cmd,event,lib}.rs
apps/depths/server/src/updates/
  mod.rs                                   # Updates service held in AppState
  store.rs                                 # updates.json / update-state.json
  creds.rs                                 # DOCKER_CONFIG reader -> oci_registry::Credentials (secrecy)
  registry.rs                              # Registry port over oci-registry; error mapping; OTel request counter
  scheduler.rs                             # check slots, windows, Auto queue, Notify wake-ups
  run.rs                                   # batch/scheduled runner over Ops; health wait; rollback; cleanup
  watchtower.rs                            # detection from the runtime snapshot; acks
apps/depths/server/src/{ops,ws,audit,config,telemetry,main,users,http}.rs   # wiring
apps/depths/compose.yaml  .env.example     # /etc/localtime mount, DOCKER_CONFIG comment
apps/depths/web/src/features/updates/      # React.lazy page: list, filters, batch preview, policy + settings sheets, Watchtower banner, recent runs
apps/depths/web/src/features/stacks/       # per-service update state on the stack page
apps/depths/web/src/app/                   # nav entry + badge (admin)
apps/depths/web/tests/support/fake-backend.ts   # update scripts
apps/depths/web/tests/e2e/us9-update-check/ us10-update-batch/ us11-update-auto/ us12-watchtower/ us13-image-cleanup/
docs/guides/depths/updates.md              # new: turning checks on, policies, windows, rollback limits
docs/guides/depths/migrating-from-watchtower.md   # new: label table, stopping Watchtower
docs/guides/depths/install.md  security.md        # mounts, credentials, keep tags
```

**Structure Decision**:

- Decision logic (references, labels, policy, windows, check, holds) is pure code in
  `depths-core::updates`, tested without Docker, a network or a clock. The server owns the I/O:
  state files, credentials, the scheduler and the runner.
- The registry client is the shared `oci-registry` crate. The `Registry` port stays in depths-core;
  depths-server implements it in `updates/registry.rs`, the same way it implements `HistorySink`
  (`file_history.rs`) and `OutputSink` (`ops.rs`) itself, rather than in a second adapter crate.
  Reading `config.json` stays in Depths (`updates/creds.rs`).
- Updates run through the existing `Ops` (lock, preview binding, `CliRuntime`, audit), not a
  parallel path. `Ops` gains a "system" `Who` for scheduled runs.

## Sequencing and prerequisites

1. **054 passkey step-up must land first** (or at least its `Preview.step_up` / `Confirm.assertion`
   server side). US2's batch confirm and every US3 policy change need it (FR-011, FR-017). US1
   (read-only) does not, and can ship first.
2. **`crates/oci-registry`:** shared with 061. Whichever spec is implemented first builds it, to
   the contract in contracts/registry.md; the other only depends on it.
3. **054 guard:** not required to ship. The hold check uses `depths-guard-core` when it exists;
   until then guard refusals are caught at `up` (R4). When 054 lands, its allowlist needs
   `POST /images/{name}/tag` (contracts/protocol.md).
4. **051:** code is in the tree (`compose_lint.rs`, `security.rs`); its tasks.md is behind. 062
   only reads its functions.
5. Suggested order: US1 (check + view, no mutation) → US2 (batch, holds) → US3 (scheduler,
   rollback, interrupted) → US4 (labels; the pure part can be built with US1) → US5 (cleanup).

## Resolved with owner (2026-10-28)

The plan's review notes, now in the spec:

- **SC-007:** checks of unchanged tags count zero pulls; each newly published image costs at most
  one counted pull, once. FR-003 says the same.
- **Private registries:** a plain `config.json` mounted into the container, with `DOCKER_CONFIG`
  set. Credential helpers are not supported (those registries show "can't check"); 050's private
  pulls have the same gap (spec Assumptions).
- **050's Update** switches to the 062 update steps while checks are on (FR-026).
- **`no-pull` label** excludes the service (FR-019, US4 scenario 8).
- **Scheduled authorisation:** the passkey-confirmed policy change authorises unattended runs;
  policies whose author is no longer an admin are paused (FR-025).
- **Rollback and migrations:** rollback can't undo migrations; `rollback_failed` waits for a
  person (FR-014, US3 scenario 7).
- **Shared tags** across the host: documented exposure (spec Edge Cases).
- **Keep tags** `depths-keep/<stack>:<svc>-previous|held`: accepted, visible in `docker images`,
  never removed by cleanup while they are a stack's current rollback target (FR-027, FR-028).
- **Registry client:** the shared `crates/oci-registry` crate replaces `oci-client`.

Plan detail, no spec change: "host local time" needs `/etc/localtime` mounted; without it windows
are UTC and the page says so.

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| Unattended mutation (scheduler) | US3 is the feature; Principle I's confirm is given once, at policy time, with a passkey | Notify-only: leaves the Watchtower use case unmet. |
| Shared `crates/oci-registry` | HEAD-only checks and digest resolution need a registry client; the daemon cannot HEAD. One crate serves Keel and Depths, on dependencies already in the tree | Daemon `/distribution` (counts as pulls every check); `oci-client` (falls back to GET when HEAD lacks a digest, a silent counted pull; a new edge for Depths); one client per app (two copies of the token flow). |
| Keep tags `depths-keep/<stack>:<service>-previous|held` | The way back must survive `docker image prune` until the next successful update | Remembering image IDs only: any prune between update and rollback removes the way back. |
| Pull by digest + tag, not `compose pull` | The image installed is the one previewed, and a held image never reaches the tag | `compose pull` moves the tag first; a held image would run on the next manual Up. |
| Two state files | Admin decisions must never be reset; observations can be | One file: a corrupt cache would force a choice between losing policies and refusing to start. |

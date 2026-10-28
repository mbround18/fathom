# Implementation Plan: Depths image updates across many stacks (Watchtower-compatible)

**Branch**: `062-depths-auto-update` | **Date**: 2026-10-28 | **Spec**: [spec.md](spec.md)

## Summary

Depths learns which services have newer images, shows them on one page, updates many stacks in one
reviewed step, and, for stacks an administrator opts in, updates them unattended in a maintenance
window with a health check and rollback. It reads Watchtower's labels so stacks labelled for
Watchtower behave the same. Research R1–R10 holds the decisions; the contracts and data model hold
the shapes.

1. **Check (US1).** An in-process registry client HEADs each service's tag and compares the digest
   with the running image's repo digests. Unchanged tags cost one HEAD and nothing else. A new
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
- **New, needs owner approval:** `oci-client` 0.18 (with `oci-spec` 0.10) in a new adapter crate
  `crates/depths-registry-oci`. Both are already in the workspace lock through Fathom
  (`src/backend/images.rs`), so no new code enters the lock; it is a new edge for Depths.
  Alternative with no new dependency: a hand-written client on `reqwest` 0.13, already in Depths'
  tree via `web-auth` (research R1, "Registry client").
- Not added: no cron or scheduler crate (own loop on `tokio::time`), no timezone crate
  (`chrono::Local` reading `/etc/localtime`), no `bollard`, no credential-helper crate.
- No new web dependency.

**Storage**:

- `<data_dir>/updates.json`: settings, policies, rejected digests, Watchtower acknowledgements.
  Admin decisions; unreadable → checks off, Auto paused, file left alone.
- `<data_dir>/update-state.json`: check results, digest resolutions, backoff, run journal.
  Observations; unreadable → moved aside and rebuilt.
- Both written atomically in the `users.rs` pattern. Local `depths-keep/<stack>` image tags on the
  Docker host for the way back (R7).

**Testing**:

- `depths-core::updates` unit suites, all deterministic: reference parsing table, label vectors
  (contracts/watchtower-labels.md, 38 cases), effective policy, window evaluation with a fake clock
  and a fake DST zone (spring-forward and fall-back days, windows crossing midnight, clock jumps),
  check/resolve logic over a fake registry that counts requests (the SC-007 budget table in
  contracts/registry.md), state transitions, keep-tag naming.
- Server: batch runner and scheduler against the existing `Mock` `ContainerRuntime` and `Fx`
  fixture in `ops.rs`, plus a scripted health sequence (healthy, unhealthy, restart loop, missing
  replica) and the fake registry; restart-mid-run → interrupted; corrupt state files; credential
  file parsing (helpers refused, nothing leaked into logs: a test captures the tracing output).
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
| II. One contract | All commands, events and shapes in `depths-protocol` with ts-rs bindings and a version bump. The registry is a port in `depths-core`; the `oci-client` adapter is a separate crate. | PASS |
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
    ├── registry.md           # Registry port, oci-client adapter, credentials, request budget
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
  registry.rs                              # Registry port, Manifest/ImageConfig, Credentials
  check.rs                                 # HEAD/compare/resolve; store detection; moved-back
  hold.rs                                  # root escalation (security::is_root_user), lint re-run
  keep.rs                                  # depths-keep tag names
crates/depths-core/src/plan.rs             # Operation::ImageUpdate / ImageRollback steps
crates/depths-core/src/ports.rs            # ContainerRuntime: image_inspect, containers_health, host_platform (default Unsupported)
crates/depths-core/src/role.rs             # CommandKind::{ViewUpdates, CheckUpdates, ManageUpdates}
crates/depths-registry-oci/                # NEW adapter crate (oci-client), owner approval
crates/depths-runtime-cli/src/lib.rs       # implement the three new port methods
crates/depths-protocol/src/{types,cmd,event,lib}.rs
apps/depths/server/src/updates/
  mod.rs                                   # Updates service held in AppState
  store.rs                                 # updates.json / update-state.json
  creds.rs                                 # DOCKER_CONFIG reader -> Credentials (secrecy)
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
- The registry client is an adapter crate behind a port, so the `oci-client` choice can be swapped
  for the `reqwest` alternative without touching the core.
- Updates run through the existing `Ops` (lock, preview binding, `CliRuntime`, audit), not a
  parallel path. `Ops` gains a "system" `Who` for scheduled runs.

## Sequencing and prerequisites

1. **054 passkey step-up must land first** (or at least its `Preview.step_up` / `Confirm.assertion`
   server side). US2's batch confirm and every US3 policy change need it (FR-011, FR-017). US1
   (read-only) does not, and can ship first.
2. **054 guard:** not required to ship. The hold check uses `depths-guard-core` when it exists;
   until then guard refusals are caught at `up` (R4). When 054 lands, its allowlist needs
   `POST /images/{name}/tag` (contracts/protocol.md).
3. **051:** code is in the tree (`compose_lint.rs`, `security.rs`); its tasks.md is behind. 062
   only reads its functions.
4. Suggested order: US1 (check + view, no mutation) → US2 (batch, holds) → US3 (scheduler,
   rollback, interrupted) → US4 (labels; the pure part can be built with US1) → US5 (cleanup).

## Spec notes (for the owner)

- **SC-007 ("zero pulls") is only reachable for unchanged tags.** Reading version labels and
  comparing the host-architecture image (FR-003, edge case "same image, new digest") needs a
  manifest GET when a tag moves, which Docker Hub counts. The plan spends at most one counted pull
  per newly published image, once, shared by every stack using it. Suggest: "checks of unchanged
  tags count zero pulls; each newly published image costs at most one, once."
- **"Credentials the host's Docker already has":** the Docker daemon holds no registry
  credentials; the CLI's `config.json` does, and Depths' container cannot see the host's. Private
  images need a mounted `config.json` (and today's 050 pulls of private images need the same).
  Credential helpers (`credsStore`) are not supported inside the container.
- **US2 scenario 5 (users updating via Pull and Up):** 050's Update is `pull` then `up`, which
  moves the tag before any hold check. The plan switches that plan to the image-update steps when
  checks are on, so the same holds apply (FR-010).
- **No-pull label:** the spec does not mention `com.centurylinklabs.watchtower.no-pull`; the plan
  excludes such services (restrictive).
- **Scheduled runs and network trust:** 052 trust is a property of a session; scheduled runs have
  none. The plan treats the trusted, passkey-confirmed policy change as the authorisation, and
  pauses Auto policies whose author stops being an administrator. Worth confirming.
- **Host time:** "host local time" needs `/etc/localtime` mounted; without it windows are UTC and
  the page says so.

## Complexity Tracking

| Addition | Why | Simpler alternative rejected |
|---|---|---|
| Unattended mutation (scheduler) | US3 is the feature; Principle I's confirm is given once, at policy time, with a passkey | Notify-only: leaves the Watchtower use case unmet. |
| `oci-client` in a new adapter crate | HEAD-only checks and digest resolution need a registry client; the daemon cannot HEAD | Daemon `/distribution` (counts as pulls every check); hand-written `reqwest` client (kept as the fallback option). |
| Keep tags `depths-keep/<stack>:<service>-previous|held` | The way back must survive `docker image prune` until the next successful update | Remembering image IDs only: any prune between update and rollback removes the way back. |
| Pull by digest + tag, not `compose pull` | The image installed is the one previewed, and a held image never reaches the tag | `compose pull` moves the tag first; a held image would run on the next manual Up. |
| Two state files | Admin decisions must never be reset; observations can be | One file: a corrupt cache would force a choice between losing policies and refusing to start. |

---
description: "Task list for spec 062: Depths image updates across many stacks (Watchtower-compatible)"
---

# Tasks: Depths image updates across many stacks (Watchtower-compatible)

**Input**: Design documents from `specs/062-depths-auto-update/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Required. Constitution III (tested at the boundaries) is non-negotiable, and the spec's
acceptance scenarios map to the tests below. In every story the test tasks come first and must fail
before the implementation tasks start.

**Gate rules for every task**:
- Run targeted tests only (quickstart §1), one command at a time, with `CARGO_BUILD_JOBS=4`.
- Subagents never run `scripts/ci.sh`, `paws` or `dagger`. One Playwright project at a time.
- The opt-in Docker test (`DEPTHS_IT_DOCKER=1`, quickstart §2) is written by agents and run by the
  main session only. It creates and removes only `depths-it-upd-<random>` stacks and their own
  images; it never touches other containers, volumes or images.
- The main session runs the full gate once, after merging.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: the user story this task serves (US1 to US5)
- **(shared with 061)**: the task builds `crates/oci-registry`, which spec 061 also uses. Whichever
  spec is implemented first does it, to contracts/registry.md; the other skips it and checks the
  crate's API still matches the contract.

## Phase 1: Setup

- [X] T001 [P] Add `us9-update-check`, `us10-update-batch`, `us11-update-auto`, `us12-watchtower` and `us13-image-cleanup` to the `STORIES` array in `apps/depths/web/playwright.config.ts`, and create the five empty directories under `apps/depths/web/tests/e2e/`. If 054 has not yet added `us7-passkeys` and `us8-guard`, leave those numbers free; renumber only if a later spec took these names.
- [X] T002 [P] Create the empty modules:
  - `crates/depths-core/src/updates/mod.rs`, declared in `crates/depths-core/src/lib.rs`;
  - `apps/depths/server/src/updates/mod.rs`, declared in `apps/depths/server/src/lib.rs`;
  - the directory `apps/depths/web/src/features/updates/`.
- [X] T003 [P] Add the `/etc/localtime:/etc/localtime:ro` mount and the commented `DOCKER_CONFIG` mount and variable to the `depths` service in `apps/depths/compose.yaml`, and the commented `DOCKER_CONFIG` line to `apps/depths/.env.example`, exactly as contracts/config.md "Compose" shows. Keep `restart: ${RESTART_POLICY:-no}` as it is.

---

## Phase 2: Foundational (blocks every story)

The shared registry client, the pure domain types, the protocol and the stores land first, in the
main session or one agent, so the story agents build on one shape.

### Shared registry client `crates/oci-registry` (shared with 061)

The crate's contract is `specs/061-keel-update-notice/contracts/registry.md`. These tasks are the
same work as 061's T001 and T004–T010: **shared with 061; skip if already built**, and then only
check that the crate's API still matches that contract.

- [X] T004 (shared with 061; skip if already built, = 061 T001) Create `crates/oci-registry/Cargo.toml` exactly as in 061's contracts/registry.md "Crate": `reqwest = { version = "0.13.5", features = ["json", "default-tls"] }`, `secrecy = "0.10"`, `serde`, `serde_json = "1.0.154"`, `sha2 = "0.11"`, `thiserror = "2"`, `tokio` (`time`), `tracing`, optional `axum = "0.8"` behind feature `test-util = ["dep:axum", "tokio/net"]`, license Apache-2.0, `publish = false`. Versions are pinned in the crate, as `web-auth` does: the workspace has no `[workspace.dependencies]` table. Add `src/lib.rs` declaring `reference`, `auth`, `client`, `manifest`, `error` and `#[cfg(feature = "test-util")] testing`. Confirm with `git diff Cargo.lock` that the only new package entry is `oci-registry`.
- [X] T005 (shared with 061; skip if already built, = 061 T004) Write unit tests in `crates/oci-registry/src/reference.rs` for every row of 061's "Reference resolution" table, and for `Digest::parse` (`sha256:` and 64 lowercase hex characters, anything else refused).
- [X] T006 (shared with 061; skip if already built, = 061 T005) Write `crates/oci-registry/src/testing.rs` (feature `test-util`): `FakeRegistry`, an axum router on `127.0.0.1:0`, with `/token`, HEAD and GET manifests, GET blobs, digests computed from fixture bodies, a switch to omit `Docker-Content-Digest`, a programmable status per path, a request log of `(method, path, had_authorization)`, and the fixture builders in 061's "Fake registry" list.
- [X] T007 (shared with 061; skip if already built, = 061 T006) Write `crates/oci-registry/tests/client.rs` against `FakeRegistry` through `Client::with_base_url`, asserting the request log and `RequestCounts` exactly, for the cases in 061's T006. For Depths it must include: `head_digest` never sends a GET, and a missing `Docker-Content-Digest` is `NoDigestHeader`; an index with an attestation entry resolves to the host platform; `NoImageForPlatform`; a digest mismatch is `BadResponse`; a 429 is `RateLimited` and still counts its HEAD; no password or token in `Debug` or captured logs.
- [X] T008 (shared with 061; skip if already built, = 061 T007–T009) Implement `crates/oci-registry/src/reference.rs` (`ImageRef`, `Reference`, `Digest`, `with_digest`), `src/error.rs` (`RegistryError` with `code()`: `invalid_reference`, `not_by_digest`, `auth_failed`, `not_found`, `rate_limited`, `timeout`, `network`, `bad_response`, `no_digest_header`, `no_image_for_platform`), `src/auth.rs` (`Credentials`, bearer challenge, token cache) and `src/manifest.rs` (index, manifest and config parsing, platform selection, `Platform::current()`), per 061's contract. T005 passes.
- [X] T009 (shared with 061; skip if already built, = 061 T010) Implement `crates/oci-registry/src/client.rs`: `Client::new` (`https` only), `Client::with_base_url` (feature `test-util`), `head_digest` and `resolve` returning `Counted<T>`, per 061's "Requests" and "Rules". T007 passes: `cargo test -p oci-registry`.

### Pure domain in `crates/depths-core/src/updates/`

- [X] T010 [P] Write the reference table tests in `crates/depths-core/src/updates/reference.rs` from data-model.md "Image reference": `nginx`, `nginx:1.27`, `library/nginx`, `docker.io/nginx`, `index.docker.io/library/nginx`, `ghcr.io/a/b:c`, `localhost:5000/x`, `host:5000/a/b@sha256:…`, `x:1@sha256:…` (both: digest wins, excluded), upper case (invalid → excluded `bad_reference`). The rules: "no registry → `docker.io`; single-component Docker Hub repo → `library/<name>`; no tag and no digest → `latest`", and "`digest.is_some()` → excluded `pinned_by_digest` (FR-004)".
- [X] T011 Implement `ImageRef` in `crates/depths-core/src/updates/reference.rs` so T010 passes. It is depths-core's own pure type; no dependency on `oci-registry`.
- [X] T012 [P] Write `crates/depths-core/src/updates/registry.rs`: the `Registry` port (`head_digest(&ImageRef) -> Result<String, RegistryError>`, `resolve(&ImageRef, &str, &Platform) -> Result<Resolution, RegistryError>`, no credentials), `Resolution`, `Platform`, and depths-core's own `RegistryError { AuthFailed, NotFound, RateLimited, Unreachable, BadResponse, NoDigestHeader, NoImageForPlatform, CredentialsUnsupported }`, per contracts/registry.md §2. No dependency on `oci-registry`. Add a `#[cfg(test)]` fake port in the same module that serves scripted digests and counts `head_digest` and `resolve` calls.
- [X] T013 [P] Write the window tests in `crates/depths-core/src/updates/window.rs` with a fake `Clock` and a fake DST `LocalZone`: a spring-forward gap at 02:00–03:00 and a fall-back repeat at 02:00–03:00. Cases from quickstart §1: window 02:30–04:00 on both days, window 23:00–01:00 across both, the clock jumping back an hour inside a window. Rules from research R5: a start in the gap starts "at the first valid instant after the gap"; an ambiguous start takes "the earliest instant"; "An occurrence whose end resolves before its start is empty, and the next occurrence is used"; each occurrence is keyed by the local date it starts on and runs once. `end <= start` crosses midnight.
- [X] T014 Implement `Window`, `Weekday`, the `LocalZone` and `Clock` traits, `next_occurrence` and occurrence keys in `crates/depths-core/src/updates/window.rs` so T013 passes. No timezone crate; `now` is always passed in.
- [X] T015 [P] Write the 38 label vectors from contracts/watchtower-labels.md "Test vectors" as a table test in `crates/depths-core/src/updates/labels.rs` (SC-006), plus the boolean table from research R6: `1 t T TRUE true True 0 f F FALSE false False`, "anything else is treated as absent and shown as 'unrecognised value'".
- [X] T016 Implement `crates/depths-core/src/updates/labels.rs` (labels → restrictions, `WatchtowerLabel`, `LabelEffect`) so T015 passes.
- [X] T017 [P] Write effective-policy tests in `crates/depths-core/src/updates/policy.rs`: "A stack with no entry is `Notify` (FR-012)"; "Restriction rank, most restrictive first: excluded, `Off`, `Notify`, `Auto`. The most restrictive of stack policy and labels wins (FR-019)"; `DepthsItself` for the container Depths runs in; every `Exclusion` variant (`PinnedByDigest, BuildContext, LocalImage, BadReference, LabelDisabled, LabelOnlyMode, ScopeMismatch { scope }, NoPull, DepthsItself`); the source is named (`StackPolicy | Default | Label(LabelRule)`).
- [X] T018 Implement `EffectivePolicy` in `crates/depths-core/src/updates/policy.rs` so T017 passes. Check `cargo test -p depths-core updates::`.

### Protocol, roles and ports

- [X] T019 [P] Write round-trip tests in `crates/depths-protocol/src/` for every new type and variant in contracts/protocol.md and data-model.md, including old-wire decoding of `StackDetail`, `AuditEntry` and `Provider` without the new optional fields.
- [X] T020 Add the types to `crates/depths-protocol/src/{types,cmd,event}.rs`:
  - `Window`, `Weekday`, `UpdateSettings`, `UpdatesView` (with `paused_policies` and `registries`), `UpdatesAvailability`, `AutoPause`, `ServiceUpdate`, `RunningImage`, `AvailableImage`, `UpdateState`, `EffectivePolicy` and its enums, `UpdatePolicy`, `StackUpdatePolicy`, `StackUpdatePlan`, `UpdateChange`, `UpdateRun`, `StackRun`, `RunPhase`, `ServiceChange`, `StackOutcome`, `WatchtowerContainer`, `WatchtowerAnswer`, `RegistryStatus`;
  - `Provider::Depths`; `AuditEntry` `on_behalf_of`, `update`, `change` (all `#[serde(default, skip_serializing_if = "Option::is_none")]`); `AuditUpdate`, `AuditServiceUpdate`, `AuditChange`; the `AuditAction` variants `image_update`, `image_rollback`, `image_hold`, `image_hold_apply`, `image_remove`, `update_interrupted`, `update_settings`, `update_policy`, `update_retry`, `watchtower_ack`;
  - `StackDetail.updates: Option<Vec<ServiceUpdate>>`;
  - the Cmds `ListUpdates`, `CheckUpdatesNow`, `PreviewUpdateBatch`, `PreviewUpdateChange`, and the Events `Updates`, `UpdatesChanged`, `UpdatesBadge`, `CheckStarted`, `UpdatePreview`, `UpdateRunStarted`, `UpdateStackDone`, `UpdateRunFinished`.
  Bump `PROTOCOL_VERSION` in `crates/depths-protocol/src/lib.rs` to the next number after 054's bump. Regenerate the ts-rs bindings into `apps/depths/web/src/protocol/` and check for drift. T019 passes.
- [X] T021 [P] Refuse the `depths` provider in `users.json` validation in `apps/depths/server/src/users.rs`, with a test (research R8: nobody can sign in as it).
- [X] T022 [P] Add `CommandKind::{ViewUpdates, CheckUpdates, ManageUpdates}` to `crates/depths-core/src/role.rs`, admin only, with role-table tests.
- [X] T023 Add `image_inspect`, `containers_health` and `host_platform` to `ContainerRuntime` in `crates/depths-core/src/ports.rs` (default `Unsupported`). Implement them in `crates/depths-runtime-cli/src/lib.rs` with parse tests over captured `docker image inspect`, `docker inspect` (`State.Health.Status`, `State.Running`, `RestartCount`) and `docker info` (platform, `driver-type`) JSON. Extend the `Mock` runtime and `Fx` fixture in `apps/depths/server/src/ops.rs` with scripted image configs, health sequences and the image store type.

### Stores, flags, wiring

- [X] T024 [P] Write store tests in `apps/depths/server/src/updates/store.rs`. Validation of `updates.json`, quoted from data-model.md:
  - "`interval_secs` ≥ 3600 (FR-002); default 21600."
  - "`health_timeout_secs` 30..=1800; default 120 (FR-014)."
  - "`window`: `start`/`end` are `HH:MM` 00:00–23:59; `days` non-empty, no duplicates; `end == start` is refused (an empty or 24 h window is ambiguous; a full day is `00:00`–`23:59`)."
  - "`scope`: `null` or 1..=64 chars `[A-Za-z0-9._-]`; the literal `none` has Watchtower's meaning."
  - "Unknown fields refused. An unreadable file is never rewritten: Depths runs with checks off and Auto paused, and Updates shows the path and the parse error (research R5)."
  Also: an unreadable `update-state.json` is moved aside to `update-state.json.unreadable-<ts>` and rebuilt; writes are atomic (temp 0600, fsync, rename, fsync dir) as in `users.rs`; the resolution cache is "Content addressed: never refreshed. Bounded to 2,000 entries, least recently used evicted."
- [X] T025 Implement `apps/depths/server/src/updates/store.rs` (`updates.json`, `update-state.json`: settings, policies, rejected, acks, per-service results, resolutions, backoff, journal, keep records) so T024 passes.
- [X] T026 [P] Add `--updates` / `DEPTHS_UPDATES`, `--updates-auto` / `DEPTHS_UPDATES_AUTO`, `--updates-check-min-gap` / `DEPTHS_UPDATES_CHECK_GAP` and the `DOCKER_CONFIG` read to `apps/depths/server/src/config.rs` through `Resolver::pick`, with resolver tests, and the one startup log line from contracts/config.md "Startup log" (never values from `config.json`).
- [X] T027 Write the `Updates` service in `apps/depths/server/src/updates/mod.rs`, held in `AppState`. With `DEPTHS_UPDATES=off`: no scheduler task, no registry client constructed, update commands answer `Unsupported`, `StackDetail.updates` is `None` (research R10). Add a test for the off path.
- [X] T028 [P] Add the spans and metrics from research R9 to `apps/depths/server/src/telemetry.rs`: `updates.check`, `updates.resolve`, `updates.run`, `updates.stack`, `updates.health_wait`; `depths.updates.registry_requests` {`registry`, `kind` = head|get|manifest_get}, `depths.updates.check_ms`, `depths.updates.outcomes`, `depths.updates.health_wait_ms`. Never credentials, paths or `auths` keys.

**Checkpoint**: `cargo test -p oci-registry`, `cargo test -p depths-core updates::`, `cargo test -p depths-protocol` and `cargo test -p depths-server updates` pass. Stories can start.

---

## Phase 3: User Story 1 - See every stack that has a newer image (P1) MVP

**Goal**: Administrators see every service's update state on one page, with a badge; users see
their assigned stacks' states. Read-only: nothing on the host changes. Ships before 054.

**Independent test**: spec US1 independent test; quickstart §1 and §3 steps 1–3.

### Tests

- [x] T029 [P] [US1] Write check tests in `crates/depths-core/src/updates/check.rs` over the fake port (T012):
  - unchanged tag: one `head_digest`, no `resolve`, state `UpToDate`;
  - moved tag: one `resolve` per new digest, however many services use it; a second pass re-uses the resolution cache;
  - multi-arch re-push with the same host-platform manifest: no update;
  - `MovedBack`, `Rejected` ("Matched on stack + service + tag digest. A newer digest for the tag makes the service `Available` again (FR-015)");
  - excluded services are never checked (FR-004);
  - `NoDigestHeader` and `CredentialsUnsupported` give `CheckFailed` with "can't check: …" and no `resolve`;
  - `RateLimited` and `Unreachable` give `CheckFailed` and keep the last known running/available fields (FR-005); a failure never reports `Available`;
  - classic vs containerd store comparison of the running image;
  - the 7-day request budget from contracts/registry.md "Request budget" for 30 stacks every 6 h (SC-007).
- [x] T030 [P] [US1] Write credential tests in `apps/depths/server/src/updates/creds.rs`, per contracts/registry.md "Credentials file": plain `auths` entries; key matching (including `https://index.docker.io/v1/` for `docker.io`); `credsStore`, `credHelpers` and `identitytoken` give `CredentialsUnsupported`; no entry means anonymous; an unreadable file means anonymous plus a panel note; captured tracing output holds no secret.
- [x] T031 [P] [US1] Write adapter tests in `apps/depths/server/src/updates/registry.rs` against `oci_registry::testing::FakeRegistry` through `Client::with_base_url`: the error mapping from `oci_registry::RegistryError` to depths-core's `RegistryError` in contracts/registry.md §3; each call's `RequestCounts` reaches the metric; the data-model reference table run through the `depths_core` and `oci_registry` parsers gives the same repository and reference, and the same registry once `docker.io` is read as `registry-1.docker.io`; a plain-http registry shows "can't check"; the host platform comes from `docker info`.
- [x] T032 [P] [US1] Write scheduler check tests in `apps/depths/server/src/updates/scheduler.rs` with the fake clock: slots `hash(stack) mod interval`; no registry request at all while `checks_enabled` is false (SC-001); Check now at most once a minute host-wide (`Refused::Busy` with `retry_after_secs`) and `Invalid` while checks are off; backoff per registry, exponential from 1 min up to the check interval (the shared client reports no `Retry-After`); no polling between slots.
- [x] T033 [P] [US1] Write gate tests in `apps/depths/server/tests/updates_ws.rs`: `ListUpdates` is admin only; `CheckUpdatesNow` needs a trusted network; a user's `StackDetail.updates` covers only their assigned stacks and they get no `UpdatesBadge`; `UpdatesChanged` is coalesced to at most one per second.
- [x] T034 [P] [US1] Write the Playwright spec `apps/depths/web/tests/e2e/us9-update-check/update-check.spec.ts`: checks-off empty state with how to turn them on; every state and reason shown (including "can't check" and Depths' own container with manual steps); filters by state and stack; the badge count; a user sees states only on their own stack page and no Updates entry; 360 px and 1280 px.

### Implementation

- [x] T035 [US1] Implement `crates/depths-core/src/updates/check.rs` (HEAD, compare, resolve, store detection, moved-back, state transitions per data-model.md "Service update state") so T029 passes.
- [x] T036 [P] [US1] Implement `apps/depths/server/src/updates/creds.rs`: read `$DOCKER_CONFIG/config.json` into `oci_registry::Credentials` (secrecy) so T030 passes.
- [x] T037 [US1] Add `oci-registry = { path = "../../../crates/oci-registry" }` to `[dependencies]` and `oci-registry = { path = "../../../crates/oci-registry", features = ["test-util"] }` to `[dev-dependencies]` in `apps/depths/server/Cargo.toml`. Implement `OciRegistry` in `apps/depths/server/src/updates/registry.rs`: the `Registry` port over `oci_registry::Client::new("depths/<version> (update-check)")`, credentials looked up per call, each `Counted` result's `RequestCounts` added to `depths.updates.registry_requests`. T031 passes.
- [x] T038 [US1] Implement the check part of `apps/depths/server/src/updates/scheduler.rs` (one task spawned in `apps/depths/server/src/main.rs` beside `Refresher`, sleeps until the next slot, woken by `tokio::sync::Notify`) so T032 passes.
- [x] T039 [US1] Handle `ListUpdates`, `CheckUpdatesNow`, `UpdatesBadge` and `StackDetail.updates` in `apps/depths/server/src/ws.rs` and `apps/depths/server/src/ws/`, with the gates in contracts/protocol.md "Gates". Handle `PreviewUpdateChange(Settings)` for `checks_enabled` and `interval_secs` only, audited `update_settings` with before and after; it uses 054's step-up when 054 is in the tree. The other settings fields arrive in US3–US5. T033 passes.
- [x] T040 [P] [US1] Add update scripts to `apps/depths/web/tests/support/fake-backend.ts`: `UpdatesView` fixtures for every `UpdateState`, the badge, checks off, and a user's `StackDetail.updates`.
- [x] T041 [US1] Write the Updates page in `apps/depths/web/src/features/updates/` (loaded with `React.lazy`): a virtualised service list (300 services under 200 ms), filters by state and stack, Check now, the checks-off empty state, registry status. Add the admin nav entry and badge in `apps/depths/web/src/app/` (`routes.tsx`, `Shell.tsx`). T034 passes for the admin path.
- [x] T042 [US1] Show per-service update state on the stack page in `apps/depths/web/src/features/stacks/`, for users and admins. T034 passes.
- [x] T043 [P] [US1] Write `docs/guides/depths/updates.md` (turning checks on, states and reasons, "can't check"), and the private-registries note from contracts/config.md "Private registries" in `docs/guides/depths/install.md`. Link it from `docs/guides/depths/README.md`.

**Checkpoint**: US1 works alone, with checks on and off, before 054.

---

## Phase 4: User Story 2 - Update many stacks in one reviewed step (P1)

**Goal**: One preview and one confirmation update many stacks, one at a time, each streamed and
audited, holds applied. Needs 054's passkey step-up.

**Independent test**: spec US2 independent test; quickstart §3 step 4.

### Tests

- [x] T044 [P] [US2] Write keep-tag naming tests in `crates/depths-core/src/updates/keep.rs`: `depths-keep/<stack>:<service>-previous` and `…-held`; "Stack and service names are lowercased and any character outside `[a-z0-9._-]` becomes `-`, with an 8-hex hash suffix when that changed the name" (research R7).
- [x] T045 [P] [US2] Write hold tests in `crates/depths-core/src/updates/hold.rs`, per research R4: a service with `user:` set is never held; old non-root to new root on a rootful daemon holds (`security::is_root_user`, `Level::Root`); on rootless/userns it is shown as `Info` (`RootRemapped`), not held; `compose_lint::lint` is re-run on the current text, refusing under `DEPTHS_COMPOSE_POLICY=enforce` and listing findings under `warn`.
- [x] T046 [P] [US2] Write plan tests in `crates/depths-core/src/plan.rs` for `Operation::ImageUpdate` and `Operation::ImageRollback`: argv `docker pull <repo>@<D>` (with `--platform` when the service sets `platform:`), `docker image tag <new image id> <compose reference>`, `docker compose up -d --no-deps --pull never <services…>` with the usual `--env-file` handling and no `--remove-orphans`; one pull for an image shared by several services; with checks on and update states present, 050's `Update` builds the image-update plan, and with checks off it stays pull-then-up (FR-026).
- [x] T047 [P] [US2] Write runner tests in `apps/depths/server/src/updates/run.rs` against `Mock` and `Fx`:
  - stacks run in order, one at a time, each audited `image_update` with `AuditUpdate`;
  - busy → `Skipped { reason: "busy: <actor> <kind>" }`; content hash changed → `Skipped { reason: "stale: preview again" }`; a failure doesn't stop the rest; `Cancel` → the rest `Skipped { reason: "cancelled" }`;
  - only services with a newer image are recreated, and the compose file is byte-identical before and after (FR-009);
  - hold: tag not moved, container unchanged, image kept as `…-held`, audited `image_hold`; a guard refusal from `up` with the old container running is `Held`, not `Failed`;
  - health wait (research R3): healthy; unhealthy early; no healthcheck, running with an unchanged `RestartCount` for the whole timeout; restart loop; missing replica; timeout;
  - rollback: retag the previous image, `up`, wait again → `RolledBack`, digest added to `rejected`, audited `image_rollback`; failing again → `RollbackFailed`, no third attempt (FR-014);
  - journal: "The phase is written (fsync) before each mutating step; an open stack run found at start becomes `Interrupted` (FR-018)", audited `update_interrupted`, with the stack's real images read from the runtime and nothing resumed or rolled back;
  - the journal "holds the open runs and the last 50 finished runs".
- [X] T048 [P] [US2] Add gate tests to `apps/depths/server/tests/updates_ws.rs`: `PreviewUpdateBatch` and its `Confirm` need admin, a trusted network and 054's step-up; a user's `PreviewOp { Update }` on an assigned stack gets the image-update plan (FR-026); `ApplyHeld` needs a passkey and is audited `image_hold_apply`; `Retry` removes the rejected entry, audited `update_retry`.
- [x] T049 [US2] Write the opt-in Docker test `apps/depths/server/tests/updates_docker.rs`, gated on `DEPTHS_IT_DOCKER=1` and skipped otherwise, per quickstart §2: a local `distribution` registry and `depths-it-upd-<random>` stacks, removed by name at the end. Cases for this story: unchanged tag is HEAD only (registry access log and `RequestCounts`) through `Client::with_base_url`; a pushed image gives one resolution and `Available`; multi-arch re-push, no update; update recreates only the changed service with the compose file byte-identical; an image that turns root is held with tag and container unchanged (SC-005); compose creates the replacement before stopping the old container (research R3); the same run on the containerd image store when `docker info` reports `driver-type io.containerd.snapshotter.v1`, skipped with a note otherwise. Run by the main session only.
- [X] T050 [P] [US2] Write the Playwright spec `apps/depths/web/tests/e2e/us10-update-batch/update-batch.spec.ts`: select stacks, preview (services, old and new digest, version labels, order, "only services with a newer image are recreated", 051 findings to acknowledge), step-up, per-stack live progress, a failed middle stack with the summary naming it, a hold with its reason and Apply anyway, a busy stack skipped.

### Implementation

- [x] T051 [P] [US2] Implement `crates/depths-core/src/updates/keep.rs` so T044 passes.
- [x] T052 [P] [US2] Implement `crates/depths-core/src/updates/hold.rs` so T045 passes. Use `depths-guard-core` for the guard check when it exists; until then rely on the `GuardRefusal` from `up` (research R4).
- [X] T053 [US2] Add `Operation::ImageUpdate { services, pins }` and `Operation::ImageRollback` to `crates/depths-core/src/plan.rs`, and switch 050's `Update` to the image-update plan while checks are on (FR-026), so T046 passes.
- [x] T054 [US2] Implement the batch runner in `apps/depths/server/src/updates/run.rs` over the `Ops` lock: pull by digest, hold checks, keep tag `…-previous`, tag, `up`, health wait, rollback, rejected digests, journal and the start-up interruption pass, outcomes and audit. Record each keep tag set in `update-state.json → keep` with the image ID. T047 passes.
- [x] T055 [US2] Handle `PreviewUpdateBatch`, its `Confirm`, `Cancel`, `PreviewUpdateChange(Retry | ApplyHeld)` and the user `PreviewOp { Update }` path in `apps/depths/server/src/ws.rs` and `apps/depths/server/src/ops.rs`, emitting `UpdateRunStarted`, `OpStarted`/`OpOutput`/`OpFinished`, `UpdateStackDone` and `UpdateRunFinished`. T048 passes.
- [x] T056 [P] [US2] Amend 054's guard allowlist: add `POST /images/{name}/tag` to `specs/054-depths-root-boundary/contracts/guard.md`, and to the guard's allowlist code if 054's guard is in the tree (contracts/protocol.md "054 contract amendment").
- [X] T057 [US2] Write the batch preview, progress and summary, held review with Apply anyway, and Retry in `apps/depths/web/src/features/updates/`, with the batch scripts in `apps/depths/web/tests/support/fake-backend.ts`. T050 passes.
- [x] T058 [P] [US2] Add "Updating stacks", holds, rollback, and the shared-tag exposure (spec Edge Cases) to `docs/guides/depths/updates.md`.

**Checkpoint**: US1 and US2 work together; manual updates are safe without any Auto policy.

---

## Phase 5: User Story 3 - Opt stacks into unattended updates (P2)

**Goal**: Auto stacks update inside the maintenance window, one at a time, rolled back when
unhealthy, audited as "Depths (scheduled)". Needs 054's passkey step-up, and ships together with
US4's Watchtower detection.

**Independent test**: spec US3 independent test; quickstart §3 steps 5 and 7.

### Tests

- [x] T059 [P] [US3] Write scheduler window tests in `apps/depths/server/src/updates/scheduler.rs` with the fake clock and DST zone:
  - Auto candidates are Auto stacks with an update available, not held, not rejected, not busy, ordered by stack name, one at a time;
  - nothing starts outside the window; when the window closes the stack in progress finishes and the rest wait (FR-013);
  - a per-stack window replaces the host window;
  - a rejected digest is not retried in the next window; a newer digest is (SC-004, FR-015);
  - Auto is paused by `DEPTHS_UPDATES_AUTO=off`, by an unreadable `updates.json`, and by an unacknowledged Watchtower container;
  - FR-025: before each stack the policy's `set_by` must still be an administrator in `users.json`; otherwise the stack is skipped and listed in `paused_policies`, "author no longer an administrator", until another admin saves it.
- [x] T060 [P] [US3] Write scheduled-run tests in `apps/depths/server/src/updates/run.rs`: the actor is `Actor { provider: depths, subject: "scheduled", username: "Depths (scheduled)" }` with `on_behalf_of` the policy's `set_by` (FR-016); a busy refusal names "Depths (scheduled)"; a held Auto stack is never applied (SC-005); `RollbackFailed` waits for a person; an interrupted scheduled run is audited with the run's actor.
- [x] T061 [P] [US3] Add gate tests to `apps/depths/server/tests/updates_ws.rs`: `PreviewUpdateChange(Policy)` and the window and `health_timeout_secs` settings need admin, a trusted network and a passkey always, and are audited `update_policy` / `update_settings` with before and after (FR-017).
- [X] T062 [P] [US3] Write the Playwright spec `apps/depths/web/tests/e2e/us11-update-auto/update-auto.spec.ts`: policy sheet (Off, Notify, Auto) with step-up; host window editor and per-stack override; the zone shown, with "windows are in UTC: mount /etc/localtime to use host time" when it is UTC; recent runs; rolled back, rollback failed in red with output, interrupted; a paused policy with "author no longer an administrator".
- [x] T063 [US3] Add the rollback case to `apps/depths/server/tests/updates_docker.rs`: an image whose healthcheck fails is rolled back within timeout + 1 min (SC-004), the tag points to the old image, `depths-keep/<stack>:<svc>-held` is present, the digest is rejected, and the next scheduled pass skips it. Run by the main session only.

### Implementation

- [x] T064 [US3] Add the window and Auto queue to `apps/depths/server/src/updates/scheduler.rs` (sleep until the next window opening, occurrence keys, `chrono::Local` as the `LocalZone`, paused policies computed each pass) so T059 passes.
- [x] T065 [US3] Run scheduled stacks through the US2 runner in `apps/depths/server/src/updates/run.rs` with the "Depths (scheduled)" actor and `on_behalf_of`, so T060 passes.
- [x] T066 [US3] Handle `PreviewUpdateChange(Policy)` and the window and health-timeout settings in `apps/depths/server/src/ws.rs`. Saving a policy makes the saver `set_by`, which lifts a pause. T061 passes.
- [X] T067 [US3] Write the policy sheet, window editor, zone note, paused-policy and recent-runs views in `apps/depths/web/src/features/updates/`, with the scripts in `apps/depths/web/tests/support/fake-backend.ts`. T062 passes.
- [x] T068 [P] [US3] Add policies, windows, rollback limits (migrations aren't undone; keep such stacks on Notify) and scheduled authorisation to `docs/guides/depths/updates.md`.

**Checkpoint**: US3 is complete, but Auto ships only once US4's Watchtower detection is merged.

---

## Phase 6: User Story 4 - Stacks labelled for Watchtower keep working (P2)

**Goal**: Watchtower labels apply without file changes, with reasons shown, and a running
Watchtower pauses Auto. The label rules themselves are in Phase 2 (T015–T018).

**Independent test**: spec US4 independent test; quickstart §3 step 6.

### Tests

- [x] T069 [P] [US4] Write detection tests in `apps/depths/server/src/updates/watchtower.rs` over the runtime snapshot: label `com.centurylinklabs.watchtower=true`; image `containrrr/watchtower` with any registry prefix, including `ghcr.io/containrrr/watchtower`, and the forks in research R6; an acknowledgement matches container ID plus image ID, so a recreated Watchtower raises the banner again; "Auto stays paused while any running Watchtower container has no matching acknowledgement"; Depths never stops or changes it (FR-021).
- [x] T070 [P] [US4] Write label-source tests in `apps/depths/server/src/updates/mod.rs`: labels read from the running container, and from `compose config --format json` when there is none; `label_only` and `scope` settings change the effective policy; changing them needs a passkey and is audited (FR-017).
- [X] T071 [P] [US4] Write the Playwright spec `apps/depths/web/tests/e2e/us12-watchtower/watchtower.spec.ts`: `enable=false`, `monitor-only=true`, `scope=media` and `no-pull=true` shown with their effective policy and source; hook and `depends-on` labels shown "not run" with the reason; the banner naming the container; acknowledging "stopped" or "monitor-only" with step-up lifts the pause.

### Implementation

- [x] T072 [US4] Implement `apps/depths/server/src/updates/watchtower.rs` (detection and acks, `PreviewUpdateChange(AckWatchtower)` audited `watchtower_ack`) so T069 passes.
- [x] T073 [US4] Feed labels into the effective policy in `apps/depths/server/src/updates/mod.rs`, and handle the `label_only` and `scope` settings in `apps/depths/server/src/ws.rs`, so T070 passes.
- [X] T074 [US4] Write the label reasons, policy source and Watchtower banner in `apps/depths/web/src/features/updates/` and `apps/depths/web/src/features/stacks/`, with the scripts in `apps/depths/web/tests/support/fake-backend.ts`. T071 passes.
- [x] T075 [P] [US4] Write `docs/guides/depths/migrating-from-watchtower.md`: the label table, what is not run, stopping Watchtower or setting it to monitor-only. Link it from `docs/guides/depths/README.md`.

---

## Phase 7: User Story 5 - Old images are cleaned up, but never the way back (P3)

**Goal**: With cleanup on, old images go after a successful update, but never the way back.

**Independent test**: spec US5 independent test.

### Tests

- [x] T076 [P] [US5] Write cleanup tests in `apps/depths/server/src/updates/run.rs`: cleanup off removes nothing (FR-023); cleanup on runs `docker image rm <old previous image id>` without `-f` after the stack is healthy; a daemon refusal is a "kept: in use" note, not a failure; each removal is audited `image_remove` with the image ID and repo digests; Depths "never removes an image it did not record as a service's previous image. No `prune`" (FR-027); an image a keep tag records as a stack's current rollback target is never removed, including after a rollback (the image rolled back to, and the `held` image).
- [X] T077 [P] [US5] Write the Playwright spec `apps/depths/web/tests/e2e/us13-image-cleanup/image-cleanup.spec.ts`: the cleanup setting with step-up, removal entries in recent runs and the audit view, "kept: in use" notes.
- [x] T078 [US5] Add the cleanup case to `apps/depths/server/tests/updates_docker.rs`: two updates with cleanup on remove only the image from two updates ago; an image used by another container is kept. Run by the main session only.

### Implementation

- [x] T079 [US5] Add the cleanup step to `apps/depths/server/src/updates/run.rs` and the `cleanup` setting to `PreviewUpdateChange(Settings)` in `apps/depths/server/src/ws.rs`, so T076 passes.
- [X] T080 [US5] Write the cleanup setting and removal entries in `apps/depths/web/src/features/updates/`, with the scripts in `apps/depths/web/tests/support/fake-backend.ts`. T077 passes.
- [x] T081 [P] [US5] Document keep tags (visible in `docker images`, what they protect, when they move; FR-028) and cleanup in `docs/guides/depths/updates.md` and `docs/guides/depths/security.md`.

---

## Phase 8: Polish and cross-cutting

- [X] T082 Run the `impeccable` skill over `apps/depths/web/src/features/updates/` and the stack page and nav it touches (critique, then polish, then harden), and apply its findings. Re-run the five update Playwright projects one at a time.
- [ ] T083 Merge the agent branches into `main`, then remove their worktrees and branches.
- [X] T084 *(2026-10-29: §2 updates_docker 8/8; §3 scripted as tests/update_walkthrough.rs, 7/7 against real Docker)* In the main session, run quickstart §2 (`DEPTHS_IT_DOCKER=1 cargo test -p depths-server --test updates_docker`) and walk quickstart §3 on the dev compose.
- [X] T085 *(2026-10-29: shared 238s, depths 509s, keel 536s at edc00edc; fathom 1603s at f63f6465 after a harness-race fix; all pass)* Run the full gate once, with `scripts/ci.sh`, in the main session. Report the wall time, the slowest steps, the delta and one speed-up idea.
- [ ] T086 Run `scripts/hygiene.sh`, then `docker builder prune --filter until=24h` if any images were built. Delete any kind cluster this work created.
- [ ] T087 (last) Make the brochure and guide screenshots of Updates (list with states, batch preview, policy sheet, Watchtower banner, phone layout) with a capture spec in `apps/depths/web/tests/promo/updates.capture.spec.ts` against the fake backend, fixture stacks only. Screen each for hostnames, emails, tokens, registry names and real stack names before committing to `docs/guides/depths/img/`.
- [ ] T088 Update `specs/062-depths-auto-update/spec.md` status to Implemented, and tick this file.

---

## Dependencies and execution order

- **Phase 1 → Phase 2**: these block everything. In Phase 2:
  - T004 → T005/T006 → T007 → T008 → T009 (the shared crate, in sequence; = 061 T001, T004–T010). Skip T004–T009 if 061 already built `crates/oci-registry`; check its API against 061's contracts/registry.md instead. If both specs are in flight, one agent owns the crate and the other waits for it. T037 needs the crate.
  - T010–T018 (depths-core) run in parallel with the crate. T020 needs T018's types settled. T023 before T024.
- **Spec 054 sequencing** (plan "Sequencing and prerequisites"):
  - **US1 can ship before 054.** It is read-only. Its one settings change (turning checks on, the interval) uses 054's step-up once 054 is in the tree.
  - **US2's batch confirm (T048, T055) and every US3 policy change (T061, T066) depend on 054's passkey step-up**: `Preview.step_up` and `Confirm.assertion`, server side at least. Don't start US2 or US3 implementation until it has merged.
  - The same holds for US4's acknowledgement and label settings (T072, T073) and US5's cleanup setting (T079), which also go through `PreviewUpdateChange` with a passkey (FR-017).
  - 054's guard is not required. T052 uses `depths-guard-core` when it exists; T056 amends 054's allowlist.
- **US3 and US4 ship together.** Auto must not run on a host before Watchtower detection (T072) is in, or both would update the same containers (FR-021).
- **US2 → US3 → US5**: US3 and US5 extend the US2 runner (`run.rs`). US4 needs only Phase 2 and US1.
- **Phase 8** runs last, and T087 is the final step before T088.
- **Shared files**: `ws.rs`, `run.rs`, `scheduler.rs`, `fake-backend.ts`, `features/updates/` and `docs/guides/depths/updates.md` are touched by several stories. Keep each agent's edits to separate match arms, functions and sections, and resolve on merge in the main session.

## Parallel examples

After Phase 2, in worktrees:

- **A, check and view (US1 server)**: T029–T033, T035–T039.
- **B, web (US1 then US2)**: T034, T040–T042, then T050 and T057 once B's US2 server shape is known.
- **C, update runner (US2 server, after 054)**: T044–T049, T051–T056.
- **D, Watchtower (US4)**: T069–T075, parallel with C.

Within a phase, every `[P]` test task can be written at once, for example in US1:

```
T029 crates/depths-core/src/updates/check.rs tests
T030 apps/depths/server/src/updates/creds.rs tests
T031 apps/depths/server/src/updates/registry.rs tests
T032 apps/depths/server/src/updates/scheduler.rs tests
T033 apps/depths/server/tests/updates_ws.rs
T034 apps/depths/web/tests/e2e/us9-update-check/update-check.spec.ts
```

## Implementation strategy

- **MVP**: Phases 1 and 2, then US1. Administrators see every stale stack on one page and users see
  their own, with no host change and no 054 dependency. It also replaces Watchtower's monitor-only
  mode.
- **Next**: US2 once 054's step-up has merged (the manual path, safe on day one). Then US3 with
  US4, shipped together. Then US5.
- Stop at each checkpoint and validate the story on its own before the next.

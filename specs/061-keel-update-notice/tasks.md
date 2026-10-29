---
description: "Task list for spec 061: Keel update notice and self-update"
---

# Tasks: Keel update notice and self-update

**Input**: Design documents from `specs/061-keel-update-notice/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Required. Constitution III (tested at the boundaries) is non-negotiable, and the plan's
testing strategy maps every acceptance scenario to a test below. Within each phase, test tasks come
first and must fail before the implementation lands. Registry tests use the in-process fake
registry (`oci_registry::testing::FakeRegistry`); nothing reaches the network. Time is paused
(`tokio::time::pause`), jitter is seeded, and the existing `Clock` is used.

**Gate rules for every task**:
- Run targeted tests only, one command at a time, with `CARGO_BUILD_JOBS=4`.
- Subagents never run `scripts/ci.sh`, `paws` or `dagger`.
- The main session runs the full gate once, after merging (T089).

**Shared with spec 062**: tasks marked **(shared with 062)** build `crates/oci-registry`. Whichever
spec is implemented first does them. If the crate already exists when 061 starts, check it against
contracts/registry.md, mark these tasks done with a note, and add only what the contract lists and
the crate lacks.

**The Ansible repository**: tasks that name `/home/mbruno/development/ansible` are changes in the
owner's repository (contracts/ansible-role.md). Nothing in this repository edits it; the owner lands
them, in the rollout order in quickstart.md.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: the user story this task serves (US1 to US5)

---

## Phase 1: Setup

- [X] T001 **(shared with 062)** Create `crates/oci-registry/Cargo.toml` exactly as in `specs/061-keel-update-notice/contracts/registry.md` ("Crate"): `reqwest = { version = "0.13.5", features = ["json", "default-tls"] }`, `secrecy = "0.10"`, `serde`, `serde_json = "1.0.154"`, `sha2 = "0.11"`, `thiserror = "2"`, `tokio` (`time`), `tracing`, optional `axum = "0.8"` behind feature `test-util = ["dep:axum", "tokio/net"]`, license Apache-2.0, `publish = false`. Add `crates/oci-registry/src/lib.rs` declaring `reference`, `auth`, `client`, `manifest`, `error` and `#[cfg(feature = "test-util")] testing`. Skip if 062 already created it. Confirm with `git diff Cargo.lock` that the only new package entry is `oci-registry`.
- [X] T002 Add `oci-registry = { path = "../../../crates/oci-registry" }` to `[dependencies]` and `oci-registry = { path = "../../../crates/oci-registry", features = ["test-util"] }` to `[dev-dependencies]` in `apps/keel/server/Cargo.toml`. Depends on T001.
- [X] T003 [P] Add the `s061` project to `apps/keel/web/playwright.config.ts` (`{ name: 's061', testDir: './tests/e2e/s061', use: { ...devices['Desktop Chrome'] } }`) and create `apps/keel/web/tests/e2e/s061/`. Check that the `s060` `testMatch` doesn't pick up `update-*.spec.ts`.

---

## Phase 2: Foundational (blocks every story)

### The shared registry crate (shared with 062)

- [X] T004 [P] Write unit tests in `crates/oci-registry/src/reference.rs` for every row of the contract's "Reference resolution" table: `docker.io` and `index.docker.io` map to `registry-1.docker.io`; a single-segment Docker Hub repository gets `library/`; no tag means `latest`; `@sha256:` gives `Reference::Digest`; the first segment is a registry only with a `.`, a `:` or `localhost`; upper-case repositories and tags over 128 characters are `InvalidReference`. Plus `Digest::parse`: `sha256:` followed by 64 lowercase hex characters, anything else refused.
- [X] T005 Write `crates/oci-registry/src/testing.rs` (feature `test-util`): `FakeRegistry`, an axum router on `127.0.0.1:0` with `/token` (issues a token; requires `Basic` when configured), HEAD and GET `/v2/<repo>/manifests/<ref>`, GET `/v2/<repo>/blobs/<digest>`, digests computed from fixture bodies, a switch to omit `Docker-Content-Digest`, a programmable status per path, and a request log of `(method, path, had_authorization)`. Fixture builders for every case in the contract's "Fake registry" list.
- [X] T006 Write `crates/oci-registry/tests/client.rs`, against `FakeRegistry` through `Client::with_base_url`, asserting the request log and `RequestCounts` exactly. Cases:
  - `head_digest` anonymous: #1 (401), #2, #3; counts `head = 2, get = 1, manifest_get = 0`; a second call with the cached token makes one HEAD;
  - `head_digest` never GETs a manifest, even when the HEAD lacks `Docker-Content-Digest`: the result is `NoDigestHeader` and the log holds no GET on `/manifests/`;
  - `Basic` credentials go only to the token endpoint, and `format!("{:?}", creds)` contains no password;
  - `resolve` on an index: #4, #5, #6, `manifest_get = 2`; the amd64 entry is chosen for an amd64 `Platform`, the attestation entry (`unknown/unknown`, `vnd.docker.reference.type`) is skipped; `Resolved` holds the index, manifest and config digests, the labels, `User` and `created`;
  - `resolve` on a single manifest: `index_digest = None`, `manifest_get = 1`; a config for another architecture is `NoImageForPlatform`;
  - an index without the platform is `NoImageForPlatform`;
  - a config without labels gives empty `labels`;
  - a 429 is `RateLimited { retry_after: None }`, and its HEAD is still counted; a 429 with `Retry-After: 120` gives `retry_after = Some(120 s)`;
  - a body whose digest doesn't match is `BadResponse`; a body over 4 MiB is `BadResponse`;
  - `resolve` on a tag is `NotByDigest` with no request made;
  - a 404 on the manifest is `NotFound`; a 401 after a token is `AuthFailed`.

  Depends on T005.
- [X] T007 Implement `crates/oci-registry/src/reference.rs` (`ImageRef`, `Reference`, `Digest`, `with_digest`, `Display` without credentials) and `crates/oci-registry/src/error.rs` (`RegistryError` with `code()`: `invalid_reference`, `not_by_digest`, `auth_failed`, `not_found`, `rate_limited`, `timeout`, `network`, `bad_response`, `no_digest_header`, `no_image_for_platform`). Makes T004 pass.
- [X] T008 Implement `crates/oci-registry/src/auth.rs`: `Credentials::{Anonymous, Basic { username, password: SecretString }}` with a redacting `Debug`; `WWW-Authenticate: Bearer` parsing; the token request (Basic only to an `https` realm, or the test base); a token cache per (realm, service, scope, username) until `expires_in` (default 60 s); token responses up to 64 KiB.
- [X] T009 Implement `crates/oci-registry/src/manifest.rs`: parse an OCI index or Docker manifest list, an OCI or Docker v2 manifest, and an image config (`os`, `architecture`, `config.Labels`, `config.User`, `created`); platform selection per contract rule 6; `Platform::current()` (`x86_64` → `amd64`, `aarch64` → `arm64`).
- [X] T010 Implement `crates/oci-registry/src/client.rs`: `Client::new` (`https` only), `Client::with_base_url` (feature `test-util`), `head_digest` and `resolve` with the exact `Accept` header, no redirects on manifests and tokens, at most 3 `https` redirects on blobs dropping `Authorization` cross-host, 4 MiB body limits, SHA-256 checks of #4–#6, a 10 s request timeout, `RequestCounts` on every `Counted<T>` (failures included), and `debug` logs of method, host, path and status only. Makes T006 pass (`cargo test -p oci-registry`).

### Keel contract (main session)

- [X] T011 [P] In `crates/keel-core/src/manifest.rs`, write the tests first, then add `params: BTreeMap<String, ParamSpec>` to `RawJob` (which stays `deny_unknown_fields`) per data-model.md "Job parameters": `params` "Optional; at most 8 entries"; name "`^[a-z_][a-z0-9_]{0,63}$`; not `ansible_*` or `keel_internal_*`"; `ParamSpec.kind` "Closed enum. `oci_digest` is `^sha256:[0-9a-f]{64}$`"; "The name may not also appear in the job's `vars` (refused at manifest parse)." Add `ParamKind::validate(&str)`. Tests: each rule refused with a named error, a valid block parses, a job without `params` parses as today.
- [X] T012 [P] In `crates/keel-exec/src/frames.rs`, add to `Frame::Assign` `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")] params: BTreeMap<String, String>`. Tests: an `Assign` without params serializes byte-identical to a fixture taken from today's encoding; one with params round-trips; the pre-061 struct shape (a copy in the test) rejects a frame with params (`deny_unknown_fields`). Update `crates/keel-exec/src/fake.rs` to carry params.
- [X] T013 [P] In `crates/keel-trust/src/challenge.rs`, add `params: BTreeMap<String, String>` to `OverrideScope`, serialized as a `params` member of the JCS object only when non-empty. Tests: the existing fixed-hash vector is unchanged; a new fixed vector with `params` hashes differently and stably.
- [X] T014 Add to `crates/keel-protocol/src/{cmd,event,types,run}.rs` everything in `specs/061-keel-update-notice/contracts/protocol.md`: commands `GetKeelBuild`, `GetUpdateStatus`, `CheckForUpdates`, `DismissUpdateDot { digest }`, `PreviewKeelUpdate { digest }`; events `KeelBuild`, `UpdateStatus`, `UpdateStatusLine` (only `update_available` and `checked_at`); types `UpdateCheckState`, `UpdateOutcome`, `UpdateError`, `AvailableImage`, `UpdatePath`; `Me` gains `keel_version` and `update_dot_dismissed`; `RunInfo` gains `params`. Write `crates/keel-protocol/tests/roundtrip_061.rs` with a round trip per new variant and per changed type.
- [X] T015 Regenerate the TS bindings with `make keel bindings` into `apps/keel/web/src/protocol/generated/`, and confirm a second run gives no diff. Depends on T014.
- [X] T016 [P] Write `apps/keel/server/migrations/0011_updates.sql` exactly per data-model.md:
  - `alter table users add column update_dot_dismissed text check (update_dot_dismissed ~ '^sha256:[0-9a-f]{64}$');`
  - `alter table runs add column params jsonb not null default '{}'::jsonb check (jsonb_typeof(params) = 'object');`
  - `create table keel_build_seen (id boolean primary key default true check (id), digest text check (digest ~ '^sha256:[0-9a-f]{64}$'), revision text, version text, seen_at timestamptz not null default now());`
  - `grant select, insert, update on keel_build_seen to keel_app; revoke delete, truncate on keel_build_seen from keel_app;`

  Extend `apps/keel/server/tests/migrate.rs` to apply it and to assert `keel_app` can't delete from `keel_build_seen`.
- [X] T017 Make `apps/keel/server/src/dispatch.rs` compile with every new command routed to a stub returning `OpError { code: "not_implemented" }`, all five in the session-only group of `authorize()` (`Denied { reason: "session_required" }` for tokens). Fill `Me.keel_version`, `Me.update_dot_dismissed` and `RunInfo.params` with defaults at their construction sites in `apps/keel/server/src/{dispatch,runs}.rs`. Depends on T014.
- [X] T018 [P] Write `apps/keel/server/src/build_info.rs`: `BuildInfo::from_env()` reading `KEEL_BUILD_VERSION`, `KEEL_BUILD_REVISION`, `KEEL_BUILD_DATE`, `KEEL_BUILD_SOURCE` and `KEEL_IMAGE_DIGEST`, per data-model.md "Running build": `version` "Semver without the `keel-v` prefix; empty is `None`"; `revision` "7–40 hex characters, optional `-dirty`"; `built_at` "RFC 3339"; `source` "`https` only"; `digest` "`sha256:` and 64 lowercase hex characters"; `platform` from `oci_registry::Platform::current()`. "A malformed value stops Keel at start, naming the variable. A missing value is `None`." Add `display()`: `0.2.0`, `untagged build abc1234`, or `development build` (research R2). Unit tests for each rule and each display form.
- [X] T019 Add the settings in `specs/061-keel-update-notice/contracts/config.md` to `apps/keel/server/src/config.rs`: `KEEL_UPDATE_CHECK` (`parse_bool`, default `false`), `KEEL_UPDATE_INTERVAL` (`parse_duration`, default `6h`, minimum `1h`), `KEEL_UPDATE_IMAGE` (default `docker.io/mbround18/keel:latest`, parsed with `oci_registry::ImageRef::parse`, refusing a digest reference and `http://`), `KEEL_UPDATE_REGISTRY_USERNAME`, `KEEL_UPDATE_REGISTRY_PASSWORD` or `_FILE` (env or file only, into `Credentials::Basic` with a `SecretString`), and `KEEL_IMAGE_DIGEST` (`--image-digest`). Validate even when checks are off. Tests for every refusal in "Validation", each naming its setting (FR-009), and that the password never appears in the startup log line.
- [X] T020 Write the skeleton of `apps/keel/server/src/updates/mod.rs`: `Updates { build: BuildInfo, config, status: watch::Sender<UpdateStatus>, cache, last_manual }`, the in-memory `UpdateStatus` per data-model.md ("`state` … Starts at `not_checked` after every restart (FR-008)"), and `AppState.updates: Arc<Updates>` built in `apps/keel/server/src/serve.rs`. No checker yet. Depends on T018, T019.
- [X] T021 [P] Write `apps/keel/web/tests/support/fake-updates.ts` and wire it into `apps/keel/web/tests/support/fake-backend.ts`: replies for `GetKeelBuild`, `GetUpdateStatus`, `CheckForUpdates`, `DismissUpdateDot` and `PreviewKeelUpdate` shaped by role, a way to push `UpdateStatus` and `UpdateStatusLine` from a test, and fixtures for each outcome and each `UpdatePath`. Depends on T015.

**Checkpoint**: `cargo test -p oci-registry`, `-p keel-core manifest`, `-p keel-exec frames`, `-p keel-trust`, `-p keel-protocol --test roundtrip_061` pass; the bindings are fresh; the migration applies on the test database; keel-server builds.

---

## Phase 3: User Story 1 - Keel says exactly what it is running (P1) 🎯 MVP

**Goal**: owners and admins see version, revision, digest and build date on the System tab; users see the version under About; the deployment pins and passes the digest.

**Independent test**: quickstart scenarios 3, 4 and 5; Playwright `update-build.spec.ts`.

### Tests

- [ ] T022 [P] [US1] Write a server DB test in `apps/keel/server/tests/updates.rs` (new): `GetKeelBuild` gives owners and admins the version, revision, `built_at` and digest, and users the version and `display` only (revision, date and digest null); `Me.keel_version` is set for every role; on start, the first run fills `keel_build_seen` without an audit entry, a start on a different digest or revision writes one `keel_updated` with `old_digest`, `new_digest`, `old_revision`, `new_revision`, `old_version`, `new_version` and actor `internal()`, and a start on the same build writes nothing.
- [ ] T023 [P] [US1] Extend `apps/keel/server/tests/system_workspace.rs`: after this spec, an admin is still refused `owner_only` on `GetSystem`, `PreviewSystemConfigure` and `PreviewSystemRemove`, and still can't see the System workspace in any list (FR-024).
- [ ] T024 [P] [US1] Extend `apps/keel/web/tests/unit/settings-tabs.test.ts`: `visibleTabs('admin')` now includes `system`; users get no tabs; `resolveTab('system', 'admin')` is `system`.
- [ ] T025 [P] [US1] Write `apps/keel/web/tests/e2e/s061/update-build.spec.ts`: the build panel for an owner and an admin (version, short revision with the full one on copy, short digest with the full one on copy, build date); "development build" and "untagged build abc1234"; a missing digest reads "unknown" with the one-line pin hint; a user sees no System tab and sees the version under About.
- [ ] T026 [P] [US1] Write `apps/keel/web/tests/e2e/s061/update-roles.spec.ts`, first part (FR-024): an admin opens System and sees the build panel and digest, and none of spec 060's sections (setup, configuration, binding, host, runs, Remove); the fake backend records that no `GetSystem` was sent for the admin; a user has no System tab and a direct `/settings/system` goes to the default tab.

### Implementation

- [ ] T027 [US1] Implement `GetKeelBuild` in `apps/keel/server/src/updates/commands.rs` (new), shaped by role as in contracts/protocol.md, and set `Me.keel_version` from `BuildInfo::display()` in `apps/keel/server/src/dispatch.rs`. Makes T022's first half pass.
- [ ] T028 [US1] In `apps/keel/server/src/updates/mod.rs`, add `record_build_seen(db, &BuildInfo)`, called once from `apps/keel/server/src/serve.rs` at start: upsert the single `keel_build_seen` row, and write `keel_updated` "once per change. On the first start after this migration, the row is filled without an audit entry". Makes T022 pass.
- [X] T029 [P] [US1] In `apps/keel/server/src/telemetry.rs`, set the resource `service.version` from `KEEL_BUILD_VERSION`, else the revision, else `CARGO_PKG_VERSION` (R9).
- [X] T030 [P] [US1] In `apps/keel/Dockerfile`'s runtime stage only: add `ARG CREATED=`, change `ARG VERSION=0.0.0` to `ARG VERSION=`, add `LABEL org.opencontainers.image.created=$CREATED`, and `ENV KEEL_BUILD_VERSION=$VERSION KEEL_BUILD_REVISION=$REVISION KEEL_BUILD_DATE=$CREATED KEEL_BUILD_SOURCE=$SOURCE` (R2). The cargo-chef and build stages stay untouched.
- [X] T031 [P] [US1] In `apps/keel/Makefile` `image`: `VERSION` from `git describe --exact-match --match 'keel-v*'` with the prefix stripped, else empty; `CREATED` from `date -u +%Y-%m-%dT%H:%M:%SZ`; pass both as build args. `image-push` keeps refusing a dirty tree.
- [X] T032 [P] [US1] Add `KEEL_IMAGE_DIGEST` to the environment passthrough in `apps/keel/compose.yaml`.
- [ ] T033 [US1] In `apps/keel/web/src/features/settings/tabs.ts`, open `system` to admins (`visibleTabs`), and update the header comment to say the System tab is owner and admin, with role-shaped sections (spec 061 FR-024). Makes T024 pass.
- [ ] T034 [US1] Write `apps/keel/web/src/features/settings/BuildPanel.tsx` (a definition list: version, revision, digest, build date, with `CopyBlock` for the full values and the "unknown" hint), and load it with `GetKeelBuild` in `apps/keel/web/src/features/settings/store.ts`.
- [ ] T035 [US1] Reshape `apps/keel/web/src/features/settings/SystemTab.tsx` by role: owners see `BuildPanel` above the existing spec 060 content; admins see `BuildPanel` only, and the tab never sends `GetSystem` or any System workspace command for them. Makes T025 and T026 pass.
- [ ] T036 [P] [US1] Show `Me.keel_version` under About in the account menu in `apps/keel/web/src/app/Shell.tsx`.

### The owner's Ansible repository (FR-003; the owner lands these)

- [X] T037 [US1] In `/home/mbruno/development/ansible/scripts/tests/test_keel_server_digest.py` (new, pytest, the repository's pattern), write the cases in contracts/ansible-role.md "Tests in that repository": digest validation; repository derivation (`repo:tag`, `registry:5000/repo:tag`, refusing `repo@sha256:…`); `digest.previous` written only on change; the rendered compose `image:` always contains `@sha256:`; check mode makes no pull.
- [X] T038 [US1] In `/home/mbruno/development/ansible/roles/keel_server/`: add `keel_server_image_digest: ""` and `keel_server_update_check: false` to `defaults/main.yml`; in `tasks/main.yml` validate the digest (`^sha256:[0-9a-f]{64}$`), derive `keel_server_image_repository`, pull a given digest with `policy: missing` or the tag with `policy: always` and read `RepoDigests`, write `digest.previous` (mode 0644, root-owned) when the target changes, and write `digest.current`; in `templates/compose.yaml.j2` use `image: {{ keel_server_target_image }}`; in `templates/keel.env.j2` add `KEEL_IMAGE_DIGEST` and `KEEL_UPDATE_CHECK={{ keel_server_update_check | lower }}`. Makes T037 pass.
- [X] T039 [US1] In `/home/mbruno/development/ansible/Makefile`, give `keel-server` the hook `$(if $(KEEL_DIGEST),-e keel_server_image_digest=$(KEEL_DIGEST))` and document `KEEL_DIGEST` in its help, for rollback with `make keel-server KEEL_DIGEST=$(cat /opt/keel/digest.previous)`.

**Checkpoint**: US1 works with the fake backend, and a locally built image shows its labels on the System tab.

---

## Phase 4: User Story 2 - Update checks are off unless turned on (P1)

**Goal**: no registry request unless `KEEL_UPDATE_CHECK=true`; when on, a check shortly after start and then every interval.

**Independent test**: quickstart scenarios 1 and 2; the scheduler tests below.

### Tests

- [ ] T040 [P] [US2] In `apps/keel/server/src/updates/mod.rs` tests, under `tokio::time::pause`, with a `FakeRegistry` and checks off: no checker task is spawned, no `oci_registry::Client` is built, and the fake's request log is empty after advancing 3 days; `CheckForUpdates` replies `OpError { code: "update_check_off" }`; `UpdateStatus.enabled` is false (FR-004, SC-001).
- [ ] T041 [P] [US2] In `apps/keel/server/src/updates/scheduler.rs` tests (new file), with an injected counting check and a seeded jitter source: the first check falls in 60–120 s after start; later checks every interval ±5%; after a failure, one retry after 5 minutes, then the normal schedule; a manual check doesn't move the schedule (FR-005, contracts/config.md "Schedule").
- [ ] T042 [P] [US2] Write `apps/keel/web/tests/e2e/s061/update-notice.spec.ts`, first part: with checks off, the owner's System tab says update checks are off and how to turn them on (`KEEL_UPDATE_CHECK=true`), with no Check now button.

### Implementation

- [ ] T043 [US2] Write `apps/keel/server/src/updates/scheduler.rs`: the schedule above, taking the check as a closure and the random source as a parameter. Spawn it from `apps/keel/server/src/serve.rs` only when `KEEL_UPDATE_CHECK` is true. Makes T040 and T041 pass.
- [ ] T044 [US2] Implement `GetUpdateStatus` in `apps/keel/server/src/updates/commands.rs`: owners get `UpdateStatus`, admins get `UpdateStatusLine`, users get `Denied { reason: "admin_only" }`; and `CheckForUpdates` returning `update_check_off` when checks are off.
- [X] T045 [P] [US2] Add `KEEL_UPDATE_CHECK`, `KEEL_UPDATE_INTERVAL`, `KEEL_UPDATE_IMAGE`, `KEEL_UPDATE_REGISTRY_USERNAME` and `KEEL_UPDATE_REGISTRY_PASSWORD_FILE` to the environment passthrough in `apps/keel/compose.yaml`, with the contract's commented example in the stack's `.env` example.
- [ ] T046 [US2] Write `apps/keel/web/src/features/settings/UpdateCard.tsx` with the off state, load the status with `GetUpdateStatus` after `Me` in `apps/keel/web/src/features/settings/store.ts`, and render the card on the owner's System tab. Makes T042 pass.

**Checkpoint**: with defaults, Keel makes no registry request; with checks on, the scheduler fires on time.

---

## Phase 5: User Story 3 - Owners learn that a newer image is published (P1)

**Goal**: the check compares digests, reads a new image once, classifies it, and pushes the result live: the dot and card to owners, one line to admins.

**Independent test**: quickstart scenarios 6 to 9; Playwright `update-notice.spec.ts` and `update-roles.spec.ts`.

### Tests

- [ ] T047 [P] [US3] In `apps/keel/server/src/updates/classify.rs` tests (new file), every row of research R1 step 5, plus `cant_tell` with `running_unknown` and `missing_labels`, and the R7 release-notes rule: a link only when the image has a version, its source is `https` and equals the running `KEEL_BUILD_SOURCE`; otherwise the `release_notes_note` text.
- [ ] T048 [P] [US3] In `apps/keel/server/src/updates/check.rs` tests (new file), against `FakeRegistry`:
  - same digest as running: `up_to_date`, the log holds HEADs only and `manifest_get = 0` (SC-005);
  - a new digest: one `resolve`, `pulls = 1`; the next check of the same digest is served from the cache, HEAD only;
  - the cache is "capped at 16 entries and evicting the oldest";
  - the running digest unknown: compared by revision and date, `digest_compare = false`;
  - an index without Keel's platform: `cant_tell` with `no_image_for_platform`;
  - 429: `last_error.code = "rate_limited"`, the last successful result kept, never `update_available` (FR-010);
  - a HEAD without `Docker-Content-Digest`: `no_digest_header`, and no GET;
  - the whole check is bounded at 30 s.
- [ ] T049 [P] [US3] Extend `apps/keel/server/tests/updates.rs`: a status change pushes `UpdateStatus` to an owner socket and `UpdateStatusLine` to an admin socket, and nothing to a user socket; `UpdateStatusLine` serializes with only `update_available` and `checked_at` (SC-006); `CheckForUpdates` is owner-only and gives `update_check_too_soon` with the retry time within a minute (FR-007); `DismissUpdateDot` stores the digest, refuses a malformed one with `invalid_digest`, and `Me.update_dot_dismissed` is null for non-owners.
- [ ] T050 [P] [US3] Write `apps/keel/web/tests/unit/updates.test.ts`: the status reducer for each `state` and `outcome`; the dot rule from data-model.md, verbatim: `role == owner && status.outcome == update_available && status.available.digest != me.update_dot_dismissed`; the admin line's wording.
- [ ] T051 [P] [US3] Extend `apps/keel/web/tests/e2e/s061/update-notice.spec.ts`: "up to date" with the check time; a pushed `update_available` shows the dot and card without a reload; Check now and its "try again at …"; dismissing hides the dot, survives a reload, and returns for a new digest, while the card stays (FR-014); `older_build` and `rebuilt` wording; a failure shows the last result, the reason and the next attempt; release-notes link and "No release notes for untagged builds".
- [ ] T052 [P] [US3] Extend `apps/keel/web/tests/e2e/s061/update-roles.spec.ts` (SC-006, FR-015): with an update available, an admin sees "A newer Keel is available; an owner can update it" and no dot, no available digest, version, revision or date, and no Check now or Update; a user sees nothing.

### Implementation

- [ ] T053 [US3] Write `apps/keel/server/src/updates/classify.rs`: classification and the release-notes URL (`<source>/releases/tag/keel-v<version>`), built server-side. Makes T047 pass.
- [ ] T054 [US3] Write `apps/keel/server/src/updates/check.rs`: `head_digest` on the watched reference, the cache, `resolve(&watched.with_digest(d), &build.platform, &creds)` for a new digest, `ImageInfo` from the labels (data-model.md), `RegistryError::code()` into `last_error`, a 30 s bound on the whole check, and the check's `RequestCounts`. Makes T048 pass.
- [ ] T055 [US3] In `apps/keel/server/src/telemetry.rs` and `check.rs`: the `keel.update_check` span with `keel.update.trigger`, `keel.update.outcome`, `keel.update.requests.head`, `keel.update.requests.get`, `keel.update.pulls`, `http.response.status_code` and `error.type`; the `keel.update.checks`, `keel.update.check.duration`, `keel.update.requests` (attribute `method`) and `keel.update.pulls` instruments as `static LazyLock`s (R9). Never the token, password or `Authorization`.
- [ ] T056 [US3] In `apps/keel/server/src/updates/mod.rs`, plug `check::run` into the scheduler and into `CheckForUpdates` (owner-only, once a minute per instance, reply `state: checking` then a push), and publish every result on the `watch` channel.
- [ ] T057 [US3] Implement `DismissUpdateDot` in `apps/keel/server/src/updates/commands.rs` (owner-only, writes `users.update_dot_dismissed`, replies `Me`), and fill `Me.update_dot_dismissed` for owners in `apps/keel/server/src/dispatch.rs`.
- [ ] T058 [US3] In `apps/keel/server/src/ws.rs`, add the `select!` arm on the `watch::Receiver::changed()` future: re-read the caller's instance role, send `UpdateStatus` to owners and `UpdateStatusLine` to admins, nothing to users (R5). Makes T049 pass.
- [ ] T059 [US3] In `apps/keel/web/src/features/settings/store.ts`, subscribe to `UpdateStatus` and `UpdateStatusLine` pushes, and add `CheckForUpdates` and `DismissUpdateDot`.
- [ ] T060 [US3] Complete `apps/keel/web/src/features/settings/UpdateCard.tsx`: up to date, update available (current and available version, revision, build date, digest, last check, release notes or the note), older build, rebuilt, can't tell with its reason, failure with the last result and next attempt, and Check now with its retry time. Stacks at 360 px.
- [ ] T061 [P] [US3] Write `apps/keel/web/src/features/settings/UpdateLine.tsx` (the admin's read-only line) and render it on the admin's System tab in `SystemTab.tsx`.
- [ ] T062 [US3] Add the dot to the System Settings entry in `apps/keel/web/src/app/nav.ts` and `apps/keel/web/src/app/Shell.tsx`, with the dot rule above and a dismiss action. Makes T050 to T052 pass.

**Checkpoint**: the notice works end to end against the fake registry and the fake backend. This is the release the owner asked for.

---

## Phase 6: User Story 4 - An owner updates Keel in one reviewed step (P2)

**Goal**: preview, passkey, a `keel-server` run pinned to the previewed digest through a repo-declared param, audit, and the restart.

**Independent test**: quickstart scenarios 11 to 15; Playwright `update-run.spec.ts`.

### Tests

- [ ] T063 [P] [US4] Write runner tests in `apps/keel/runner/src/executor/mod.rs`: with declared params, `params.json` is written with mode 0600 and passed as `-e @params.json` after `secret_vars.yaml` and before `forced.json`; an undeclared name, a wrong kind or a malformed value refuses with `run_refused { reason: "params_mismatch" }`; a run without params runs exactly as today.
- [ ] T064 [P] [US4] Write a runner test in `apps/keel/runner/src/executor/signed.rs`: the host rebuilds the override challenge with the assigned params, and an approval made without them doesn't verify.
- [ ] T065 [P] [US4] Write a server DB test in `apps/keel/server/tests/update_run.rs` (new):
  - `NewRun` refuses a param the job doesn't declare, or a value that doesn't match its kind, before the run is created;
  - `PreviewKeelUpdate` refusals, each audited as `keel_update_refused` with its `reason`: `owner_only` (admin), `session_required`/`token_caller` (API token), `no_system_workspace`, `no_keel_host`, `profile_not_allowed`, `job_missing_param`, `digest_stale`;
  - confirm with step-up creates a run with `trigger = 'manual'`, `params = {"keel_server_image_digest": "<new>"}`, and audits `keel_update_requested` with `old_digest`, `new_digest`, `old_revision`, `new_revision`, `run`, `workspace`;
  - a second request while one is pending, queued or running replies `Run` with the existing run and audits `update_in_progress` (FR-018), using the data-model query under the insert's transaction;
  - a status that moved between preview and confirm gives `digest_stale`;
  - the run finishing audits `keel_update_result` with `run`, `outcome`, `old_digest`, `new_digest`;
  - `RunInfo.params` carries the digest; older runs have `{}`.
- [ ] T066 [P] [US4] Write `apps/keel/web/tests/e2e/s061/update-run.spec.ts`: Update opens a preview naming host, profile, current and new digest and the restart; `older_build` and `rebuilt` show the warning line; confirming asks for a passkey (the soft passkey); the run shows in Runs with its params; pressing Update again shows the existing run; a failed run leaves the card saying the update failed, with a link to the run; an admin and a user never see Update.

### Implementation

- [ ] T067 [US4] In `apps/keel/server/src/runs.rs`, add `NewRun.params`, validated against the job's declared params (`keel_core::manifest`), stored in `runs.params`, and returned in `RunInfo.params`.
- [ ] T068 [US4] In `apps/keel/server/src/queue.rs`, send `params` in `Frame::Assign` and in the `OverrideScope` of signed approvals. A runner that rejects the frame fails the run with `runner_outdated`, as for any rejected `Assign`.
- [ ] T069 [US4] In `apps/keel/runner/src/executor/mod.rs`, re-read the job's declared params at the pinned commit, refuse on mismatch, write `params.json` (0600) and add `-e @params.json` in that order; in `apps/keel/runner/src/executor/signed.rs`, include params when rebuilding the challenge. Makes T063 and T064 pass.
- [ ] T070 [US4] In `apps/keel/server/src/system.rs`, add `update_path()`: `UpdatePath::Run` when a System workspace exists, an enrolled host has the `keel-server` profile, `keel-server` is in `allowed_profiles`, and the bound repo's `keel-server` job declares `params.keel_server_image_digest`; otherwise `Manual` with `missing` set to the first failing code. Include it, and `update_run`, in `UpdateStatus`.
- [ ] T071 [US4] Implement `PreviewKeelUpdate` and its confirm in `apps/keel/server/src/updates/commands.rs`: owner-only, session-only, step-up; the preview text per contracts/protocol.md (old and new version, revision and digest; workspace, host, job; approval policy; a warning line for `older_build` and `rebuilt`); every condition re-checked on confirm; the FR-018 query `select id from runs where workspace_id = $system and params ? 'keel_server_image_digest' and state in ('pending_approval', 'queued', 'running') for update` in the same transaction as the insert; every refusal audited as `keel_update_refused`. Audit the token refusal in `apps/keel/server/src/dispatch.rs` `authorize()`.
- [ ] T072 [US4] When an update run finishes (success, failure, cancelled or `runner_lost`), write `keel_update_result` in `apps/keel/server/src/runs.rs`. Makes T065 pass.
- [ ] T073 [US4] Add Update to `apps/keel/web/src/features/settings/UpdateCard.tsx` when `update_path.kind == "run"`: the preview and step-up flow through the existing preview components, the existing-run case, the in-progress state from `update_run`, and the failed state with a link to the run.
- [ ] T074 [P] [US4] Show a run's params next to its commit in `apps/keel/web/src/features/runs/RunPage.tsx` and `parts.tsx`. Makes T066 pass.

### The owner's Ansible repository (after the rollout steps; the owner lands these)

- [X] T075 [US4] In `/home/mbruno/development/ansible/scripts/tests/test_keel_manifest_check.py`, add cases: the `keel-server` job's `params:` block is accepted; more than 8 params, a bad name, an `ansible_*` or `keel_internal_*` name, a kind other than `oci_digest`, and a name also in `vars` are each refused.
- [X] T076 [US4] In `/home/mbruno/development/ansible/scripts/keel-manifest-check.py`, add `params` to `JOB_KEYS` and check its shape as above. Makes T075 pass.
- [ ] T077 [US4] Only after quickstart.md rollout steps 1 to 4 (Keel and the runner with params support deployed on Keel's host): add `params: { keel_server_image_digest: { kind: oci_digest } }` to the `keel-server` job in `/home/mbruno/development/ansible/.keel/jobs.yaml`. Run `make lint` there.

**Checkpoint**: an owner updates Keel from the card on a test instance (quickstart 11 to 13), and rollback by `KEEL_DIGEST` works (15).

---

## Phase 7: User Story 5 - Without a System workspace, the owner gets instructions (P3)

**Goal**: the card shows the digest reference and copyable steps, and names the missing step.

**Independent test**: quickstart scenario 10; Playwright `update-manual.spec.ts`.

### Tests

- [ ] T078 [P] [US5] Write a server DB test in `apps/keel/server/tests/update_manual.rs` (new): for each of `no_system_workspace`, `no_keel_host`, `profile_not_allowed` and `job_missing_param`, `UpdateStatus.update_path` is `Manual` with that `missing`, `digest_ref` is `mbround18/keel@sha256:…` for the available digest, and `steps` holds the deployment-repository step (`make keel-server KEEL_DIGEST=<digest>`) and the plain-compose steps (pin `image:` to the reference, `docker compose pull`, `docker compose up -d`).
- [ ] T079 [P] [US5] Write `apps/keel/web/tests/e2e/s061/update-manual.spec.ts`: each missing prerequisite names itself; the digest reference and steps are copyable; there is no Update button; a link goes to the System tab setup (FR-021).

### Implementation

- [ ] T080 [US5] Write `apps/keel/server/src/updates/manual.rs`: render `digest_ref` (repository from the watched reference, plus the available digest) and `steps`, server-side, and use it in `system.rs` `update_path()`. Makes T078 pass.
- [ ] T081 [US5] Add the manual section to `apps/keel/web/src/features/settings/UpdateCard.tsx`: what is missing in one line, the steps in `CopyBlock`s, and the setup link. Makes T079 pass.

**Checkpoint**: every story works on its own against the fake backend.

---

## Phase 8: Polish and cross-cutting

- [X] T082 [P] Write `docs/guides/keel/updates.md`: turning checks on (`KEEL_UPDATE_CHECK`, interval, image, credentials), what the card's states mean, updating from the card, the manual path, rollback with `KEEL_DIGEST` and `digest.previous`, and the rollout order for params. Link it from `docs/guides/keel/README.md`.
- [X] T083 [P] Write the crate docs in `crates/oci-registry/src/lib.rs` (what it is, the HEAD-first rule, `test-util`), and add `crates/oci-registry` to the architecture section of `CONTRIBUTING.md` as the registry adapter shared by Keel and Depths.
- [ ] T084 Extend `apps/keel/web/tests/e2e/polish-screens.spec.ts` with the System tab as owner (build panel and card in each state) and as admin (build panel and line), at 360 and 1280 px, in light and dark, asserting no horizontal scroll.
- [ ] T085 Run the `impeccable` skill over the UI changes and their surroundings (critique, then polish, then harden): `apps/keel/web/src/features/settings/{SystemTab,BuildPanel,UpdateCard,UpdateLine}.tsx`, the nav dot in `apps/keel/web/src/app/{nav.ts,Shell.tsx}`, and the params display in `apps/keel/web/src/features/runs/`. Apply its findings and rerun the `s061` project.
- [ ] T086 Walk through quickstart.md scenarios 1 to 10 and 14 against the dev stack and the fake registry; scenarios 11 to 13 and 15 on the test instance after the rollout steps.
- [ ] T087 Merge the agent branches into `main`, then remove their worktrees and branches.
- [ ] T088 Run `make keel bindings` once more on the combined branch and confirm no diff.
- [ ] T089 Run the full gate once, with `scripts/ci.sh`, in the main session only. Report the wall time, the slowest steps, the delta and one speed-up idea.
- [ ] T090 Run `scripts/hygiene.sh`, then `docker builder prune --filter until=24h` if any images were built.
- [ ] T091 Update `specs/061-keel-update-notice/spec.md` status to Implemented, and tick this file.

---

## Dependencies and execution order

### Phases

- **Phase 1 → Phase 2**: these block everything. T001 → T002. T003 is independent.
- **Phase 2**:
  - The crate: T004 and T005 in parallel, then T006; T007 → T008 → T009 → T010 (T007–T009 touch separate files and can overlap once T005's fixtures exist). Skipped if 062 built the crate.
  - The Keel contract: T011, T012, T013, T016, T018 in parallel; T014 → T015 → T021; T014 → T017; T018 + T019 → T020.
- **After Phase 2**, the stories:
  - **US1** depends only on Phase 2.
  - **US2** depends on Phase 2. Its card (T046) builds on US1's System tab reshape (T035).
  - **US3** depends on US2's scheduler and `GetUpdateStatus` (T043, T044).
  - **US4** depends on US3 (it acts on `status.available`). Its runner and run-params tasks (T063, T064, T067–T069) depend only on Phase 2 and can start early.
  - **US5** depends on US4's `update_path()` (T070).
- **Phase 8** runs last. T085 runs after the web work merges; T089 after T087.

### Ansible repository

- T037 → T038 → T039 can land at any time; they don't touch `.keel/jobs.yaml`.
- T075 → T076 → T077 land only after Keel and the runner with params support are deployed on Keel's host (quickstart.md rollout steps 1 to 4). Until then, the card shows the manual path with `job_missing_param`.

### Within each story

- Tests first, and failing, then the implementation.
- Server before web where the web consumes a new reply; the fake backend (T021) lets web work start without the server.

### Parallel agents (in worktrees, after Phase 2)

- **A, update check**: T040, T041, T043, T044, T047, T048, T053–T056 (`apps/keel/server/src/updates/`, `telemetry.rs`).
- **B, server commands, push and the update run**: T022, T023, T027, T028, T049, T057, T058, T065, T067, T068, T070–T072, T078, T080.
- **C, runner params**: T063, T064, T069.
- **D, web**: T024–T026, T033–T036, T042, T046, T050–T052, T059–T062, T066, T073, T074, T079, T081.
- **E, image and docs**: T029–T032, T045, T082, T083.
- **Shared files**: `dispatch.rs` (T017, T027, T057, T071), `updates/commands.rs` (T027, T044, T057, T071) and `UpdateCard.tsx` (T046, T060, T073, T081). Keep each agent's edits to separate match arms and functions, and resolve on merge in the main session.

---

## Parallel examples

### Phase 2

```text
Task: "Unit tests for ImageRef and Digest in crates/oci-registry/src/reference.rs"        (T004)
Task: "FakeRegistry in crates/oci-registry/src/testing.rs"                                (T005)
Task: "Manifest params in crates/keel-core/src/manifest.rs"                              (T011)
Task: "Assign.params in crates/keel-exec/src/frames.rs"                                  (T012)
Task: "OverrideScope.params in crates/keel-trust/src/challenge.rs"                       (T013)
Task: "Migration apps/keel/server/migrations/0011_updates.sql"                           (T016)
Task: "BuildInfo in apps/keel/server/src/build_info.rs"                                  (T018)
```

### User Story 1

```text
Task: "Server DB test apps/keel/server/tests/updates.rs"                                 (T022)
Task: "Owner-only checks in apps/keel/server/tests/system_workspace.rs"                  (T023)
Task: "Vitest apps/keel/web/tests/unit/settings-tabs.test.ts"                            (T024)
Task: "Playwright apps/keel/web/tests/e2e/s061/update-build.spec.ts"                     (T025)
Task: "Playwright apps/keel/web/tests/e2e/s061/update-roles.spec.ts"                     (T026)
# then, alongside the server work:
Task: "Dockerfile runtime stage in apps/keel/Dockerfile"                                 (T030)
Task: "Makefile image target in apps/keel/Makefile"                                      (T031)
Task: "KEEL_IMAGE_DIGEST in apps/keel/compose.yaml"                                      (T032)
```

### User Story 3

```text
Task: "Classification tests in apps/keel/server/src/updates/classify.rs"                 (T047)
Task: "Check tests in apps/keel/server/src/updates/check.rs"                             (T048)
Task: "Push and dismiss tests in apps/keel/server/tests/updates.rs"                      (T049)
Task: "Vitest apps/keel/web/tests/unit/updates.test.ts"                                  (T050)
Task: "Playwright update-notice.spec.ts and update-roles.spec.ts"                        (T051, T052)
```

### User Story 4

```text
Task: "Runner params tests in apps/keel/runner/src/executor/mod.rs"                      (T063)
Task: "Runner challenge test in apps/keel/runner/src/executor/signed.rs"                 (T064)
Task: "Server DB test apps/keel/server/tests/update_run.rs"                              (T065)
Task: "Playwright apps/keel/web/tests/e2e/s061/update-run.spec.ts"                       (T066)
```

---

## Implementation strategy

- **MVP**: Phases 1 and 2, then the three P1 stories: US1 (Keel says what it runs), US2 (checks
  off by default) and US3 (the notice). Stop and validate with quickstart scenarios 1 to 9. That
  is the feature the owner asked for: an owner learns a newer image exists and can update by hand.
  US1 alone is a smaller first cut if needed: the build panel answers "which build?" on its own.
- **Next**: US4 (one-click update), whose runner and run-params half can start in parallel with
  US3. It is usable once the rollout steps in quickstart.md are done in the Ansible repository.
- **Then**: US5 (manual instructions), then Phase 8.
- **Ordering across repositories**: ship Keel and the runner first; the Ansible repository adds
  `params:` last (T077).

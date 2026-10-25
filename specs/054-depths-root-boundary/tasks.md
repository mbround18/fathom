---

description: "Task list for the Depths root boundary and passkeys (054)"
---

# Tasks: Depths root boundary and passkeys

**Input**: Design documents from `/specs/054-depths-root-boundary/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/guard.md](./contracts/guard.md),
[contracts/protocol.md](./contracts/protocol.md), [contracts/config.md](./contracts/config.md),
[quickstart.md](./quickstart.md)

**Tests**: REQUIRED. Constitution Principle III is non-negotiable, so:

- each new protocol variant gets a round-trip test;
- each guard rule gets a row in the rule table that fails without the rule, written before the
  rule;
- each user story gets Playwright E2E against the fake backend (projects `us7-passkeys` and
  `us8-guard`) or a Rust test against a fake upstream.

**Organization**:

- Setup and Foundational create the two new crates, the protocol types and the audit actions.
- US1 builds the guard end to end: rules, proxy, topology, and how refusals reach Depths.
- US2 adds exceptions to it.
- US3 moves the helper into its own image.
- US4 adds passkeys and verified sessions.
- US5 adds step-up.

US1/US2, US3 and US4/US5 are three independent tracks.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel: different files, and no dependency on an incomplete task.
- **[Story]**: US1–US5, used in the user-story phases only.

## Rules for every task in this file

- **Builds and tests**:
  - Run them with `CARGO_BUILD_JOBS=4 nice`, one heavy command at a time, using
    `cargo check -p <crate>` while iterating.
  - Gate on the targeted tests named in the task and in quickstart.md §1.
- **CI**: subagents never run `scripts/ci.sh`, `paws` or `dagger`. The main session runs the full
  gate once, on the combined branch.
- **Docker**: only T079 and T080 (opt-in, `DEPTHS_IT_DOCKER=1`) touch a real daemon.
  - They remove what they created, by name (`depths-it-<random>` networks and containers).
  - Never prune, never remove an image, and never touch another container or volume.
- **Protocol changes**: after any edit to `crates/depths-protocol`, run
  `cargo test -p depths-protocol --test export_bindings`, and commit the regenerated bindings with
  the change.
- **Never log**:
  - The server never logs paths, and never puts them in span attributes.
  - Nobody logs or audits passkey credentials, challenges or recovery codes.
- **Fail closed**: every new branch that cannot decide refuses. A test that "allows on error" is a
  bug.
- **Compose**: every service keeps `restart: ${RESTART_POLICY:-no}`.
- **Commits**: one signed commit per task or tight group of tasks.

---

## Phase 1: Setup

- [ ] T001 Create the agnostic crate `crates/depths-guard-core`.
  - `Cargo.toml`: `edition = "2024"`, `license = "Apache-2.0"`, `publish = false`.
  - Dependencies: `depths-core`, `serde`, `serde_json`, `toml`, `thiserror`. No tokio, no hyper,
    no filesystem I/O.
  - Empty modules `endpoint`, `body`, `rules`, `config`, `exception` in
    crates/depths-guard-core/src/lib.rs. The existing `crates/*` member picks it up.
- [ ] T002 Create the binary crate `depths-guard` in apps/depths/guard/, and add
  `"apps/depths/guard"` to the root `Cargo.toml` `[workspace] members`.
  - Files: `src/main.rs`, `serve.rs`, `proxy.rs`, `lookup.rs`, `paths.rs`, `log.rs`, `check.rs`,
    `health.rs`.
  - Dependencies: `depths-guard-core`, `tokio`, `hyper` 1, `hyper-util`, `http-body-util`,
    `rustix` (`fs`), `serde_json`, `tracing`, `clap`. All are workspace versions.
  - Subcommands `serve` (default), `health` and `check` are stubs that exit 2.
- [ ] T003 [P] Add `webauthn-rs` 0.5 to apps/depths/server/Cargo.toml.
  - Features: `danger-allow-state-serialisation` only if ceremony state must be stored. Prefer
    in-memory and leave it off.
  - Add `openssl = { version = "0.10", features = ["vendored"] }`.
  - Add `webauthn-authenticator-rs` (softpasskey) as a dev-dependency.
  - Add the OpenSSL licence (Apache-2.0) to deny.toml if `cargo deny check licenses` asks for it.
  - Confirm `cargo check -p depths-server` builds.
- [ ] T004 [P] Add the 054 settings to apps/depths/server/src/config.rs:
  - `DEPTHS_STEP_UP_WINDOW` (seconds, default 300, clamped to 30–3600 with a startup warning);
  - `DEPTHS_GUARD_URL` (derived from `DOCKER_HOST` when unset);
  - `DEPTHS_HELPER_IMAGE` becomes **required**, and start fails naming the key when it is unset.

  Add unit tests in the same file for the default, the clamp at each end, the derivation, and the
  missing helper image.
- [ ] T005 [P] Register the Playwright projects `us7-passkeys` and `us8-guard` in
  apps/depths/web/playwright.config.ts by appending them to `STORIES`.
  - Create apps/depths/web/tests/e2e/us7-passkeys/ and apps/depths/web/tests/e2e/us8-guard/, each
    with a placeholder spec that only opens the stacks page.

---

## Phase 2: Foundational (blocks every story)

**Purpose**: the shared types every story uses. These are the protocol variants, the audit actions,
the risky-command classification, the guard-refusal parser, and the shared build id.

- [ ] T006 Add the protocol types of contracts/protocol.md to crates/depths-protocol/src/types.rs,
  with ts-rs bindings:
  - `PasskeyRequest`, `PasskeyResponse`, `CeremonyId`, `PasskeyInfo`, `Verification`;
  - `GuardRefusal { rule, setting, service: Option<String> }`;
  - `GuardException`, `GuardGrant`, `GuardGrantKind` (`device|bind|cap|namespace`);
  - `PasskeyPurpose::{Register { name }, Verify}`;
  - `PasskeyRefusedReason` (`replayed | wrong_origin | unknown_credential | expired | cancelled |
    recovery_invalid`).
- [ ] T007 Add the commands and events of contracts/protocol.md.
  - In crates/depths-protocol/src/cmd.rs:
    - new: `GetVerification`, `PasskeyBegin`, `PasskeyFinish`, `ListPasskeys`,
      `PreviewPasskeyRemove`, `RecoverPasskeys`, `GetGuardExceptions`;
    - `Confirm` and `OpenShell` gain `#[serde(default)] assertion: Option<PasskeyResponse>`.
  - In crates/depths-protocol/src/event.rs:
    - `Preview.step_up`;
    - the refusals `StepUpRequired`, `PasskeyEnrolmentRequired`, `PasskeyRefused` and
      `RuntimeGuardUnreachable`;
    - `OpFinished.guard`, `FileRefused::Guard`, and the replies `PasskeyChallenge`,
      `PasskeyRegistered`, `Passkeys`, `GuardExceptions`.
  - Bump the protocol version in crates/depths-protocol/src/lib.rs.
- [ ] T008 Add round-trip tests for every variant from T006–T007 to
  crates/depths-protocol/tests/roundtrip.rs.
  - Include an old-shape `Confirm` and an old-shape `OpenShell`, both without `assertion`, which
    must still parse.
  - Regenerate the bindings with `--test export_bindings`.
- [ ] T009 [P] Add the audit actions to `AuditAction` in crates/depths-protocol/src/types.rs, and
  their display in apps/depths/server/src/audit.rs:
  - `GuardRefused { rule, setting, service? }`;
  - `PasskeyRegistered { name }`, `PasskeyRemoved { name }`;
  - `PasskeyVerified`, `PasskeyFailed { reason }`;
  - `StepUpRequired { action }`;
  - `PasskeyRecoveryIssued`, `PasskeyRecovered`.

  Add a test asserting that no variant has a field holding a credential, challenge or code.
- [ ] T010 [P] Expose `DEFAULT_CAPS` and `pub fn is_default_cap(&str) -> bool` in
  crates/depths-core/src/security.rs (normalised through `normalise_cap`).
  - The set is CHOWN, DAC_OVERRIDE, FSETID, FOWNER, MKNOD, NET_RAW, SETGID, SETUID, SETFCAP,
    SETPCAP, NET_BIND_SERVICE, SYS_CHROOT, KILL and AUDIT_WRITE.
  - Make `cap_reasons` use it so the lint and the guard share one list.
  - Unit tests: each default cap, `cap_` and lowercase forms, and `SYS_ADMIN` not default.
- [ ] T011 [P] Add `pub fn step_up_needed(kind: CommandKind, escape_findings: bool) -> bool` to
  crates/depths-core/src/role.rs, beside `restricted_ok`, covering the FR-017 list:
  - shell open;
  - a stack save or deploy with escape findings;
  - acknowledging an override;
  - file write, upload, rename, delete and restore;
  - user, role and network-whitelist changes;
  - passkey removal.

  Add any new `CommandKind` variants these need. Table test in the same file: every
  `CommandKind`, with and without findings.
- [ ] T012 [P] Parse guard refusals in crates/depths-runtime-cli/src/lib.rs.
  - `pub fn guard_refusal(stderr: &str) -> Option<GuardRefusal>` matches
    `depths-guard refused <rule>: <setting> [<project>/<service>]`. The bracket part is optional.
  - `pub fn guard_unreachable(stderr: &str) -> bool` matches connection-refused to the guard
    address and the guard's own `depths-guard: runtime unreachable`.
  - Unit tests: compose and CLI wrappings (`Error response from daemon: ...`), the bracket present
    and absent, a setting containing `:` and spaces, and unrelated errors returning `None`.
- [ ] T013 [P] Add `pub const BUILD_ID: &str = match option_env!("DEPTHS_BUILD_ID") { Some(s) => s,
  None => "dev" };` to crates/depths-volumes/src/lib.rs, so the server and the helper read one
  constant.

**Checkpoint**: protocol bindings regenerated, `cargo test -p depths-protocol -p depths-core -p
depths-runtime-cli` green.

---

## Phase 3: User Story 1 - The runtime refuses escapes even when Depths asks (Priority: P1) 🎯 MVP

**Goal**: a guard between Depths and the socket proxy. It denies by default, refuses every
escape-level request, and logs the refusals where Depths cannot reach. Ordinary stacks deploy
unchanged.

**Independent Test**: `cargo test -p depths-guard-core` (rule table) and `cargo test -p
depths-guard` (fake upstream). Every FR-003/FR-005 case is refused, never forwarded, and logged.
The example stacks' recorded requests are forwarded byte for byte, and with the upstream down,
nothing is forwarded.

### Tests for User Story 1 (write first, confirm they fail)

- [ ] T014 [P] [US1] Endpoint classification tests in crates/depths-guard-core/src/endpoint.rs.
  - Every `Endpoint` variant in data-model.md, with and without the `/v1.NN` prefix.
  - Query strings ignored.
  - `/build` and `/session` map to `Build`.
  - `archive`, `export`, `commit`, `plugins/*`, `swarm`, `secrets`, `configs`, `nodes`,
    `services`, `images/load` and `images/*/push` map to `Unknown`.
  - Path traversal tricks (`/containers/../plugins`, encoded slashes) map to `Unknown`.
- [ ] T015 [P] [US1] Rule-table tests in crates/depths-guard-core/tests/rules.rs: one row per rule
  in data-model.md "Rule", each a create body that must be refused with that rule name.
  - **Namespaces:** `Privileged`; each of Pid, Ipc, UTS, Userns, Cgroupns and Network mode set to
    `host`.
  - **Capabilities:** `CapAdd: ["ALL"]`; `CapAdd: ["SYS_ADMIN"]`; the `Capabilities` field.
  - **Devices:** `Devices`, `DeviceRequests`, `DeviceCgroupRules`.
  - **Security options:** `seccomp=unconfined`, `apparmor=unconfined`, `label=disable`,
    `label:disable`, `systempaths=unconfined`; a non-empty `MaskedPaths` or `ReadonlyPaths`; a
    non-default `Runtime`.
  - **Host paths:** a bind of `/etc`; a long-form `Mounts` bind of `/etc`;
    `/opt/stacks/../etc`; `//var//run/docker.sock`; a bind of `/var/run/docker.sock` under an added
    root; a writable sensitive path under an added root.
  - **Protected objects:** `VolumesFrom` a protected container; `NetworkMode:
    container:<protected>`; `EndpointsConfig` with a protected network.
  - **Bodies:** an unknown `HostConfig` field; bad JSON; a body over 1 MiB.

  Allowed rows:
  - the example stacks' create bodies, as fixtures in crates/depths-guard-core/tests/fixtures/;
  - a named volume;
  - a bind under `stacks_dir`;
  - a read-only bind of a non-sensitive path under an added root;
  - default caps.

  Each refusal's `setting` names the field and value. This is SC-001.
- [ ] T016 [P] [US1] Update, exec and network rule tests in crates/depths-guard-core/tests/rules.rs:
  - update with `Devices`, `DeviceRequests` or `DeviceCgroupRules` is refused;
  - exec with `Privileged: true` is refused, as is exec with a non-empty `User`, and exec into a
    protected container;
  - network connect, create (same name) or remove of a protected network is refused;
  - volume create with local-driver `type=none,o=bind,device=/etc` is refused, but with
    `device=<stacks_dir>/x` it is allowed after the path walk.
- [ ] T017 [P] [US1] `NeedsUpstream` tests in crates/depths-guard-core/tests/rules.rs:
  - a create naming volume `v` returns `NeedsUpstream(VolumeInspect(v))`, and judging again with a
    bind-options volume refuses `volume_bind_options`;
  - a bind source returns `NeedsUpstream(PathWalk)`, and judging again with
    `symlinked_path`/`missing_bind_source` facts refuses;
  - a start returns `NeedsUpstream(ContainerInspect)`, and the stored config is judged as a
    create;
  - judging with a missing fact never returns `Allow`.
- [ ] T018 [P] [US1] Proxy tests in apps/depths/guard/tests/proxy.rs. An in-process hyper fake
  upstream records what it receives. Check that:
  - refused requests get a 403 with the contracts/guard.md body, and the upstream receives nothing;
  - allowed bodies arrive byte-identical;
  - an `Unknown` endpoint is refused;
  - an attach or exec start upgrade is piped both ways;
  - streaming logs and events are not buffered (the first chunk arrives before the upstream ends);
  - with the upstream down, the answer is 502 `depths-guard: runtime unreachable`, and nothing is
    allowed around it (SC-003);
  - a refusal writes one JSON log line with `time, endpoint, project, service, outcome, rule,
    setting`, and an allowed request without an exception writes none;
  - `/_depths_guard/v1/health` works and any other `/_depths_guard/` path is 404.
- [ ] T019 [P] [US1] Path-walk tests in apps/depths/guard/tests/paths.rs, against a temporary
  directory used as the root:
  - a plain directory is allowed;
  - a symlink at any component is refused `symlinked_path`;
  - `..` escaping the root is refused;
  - a missing source is refused `missing_bind_source`;
  - an absolute symlink target is refused.

### Implementation for User Story 1

- [ ] T020 [US1] Implement `classify(method, path) -> Endpoint` in
  crates/depths-guard-core/src/endpoint.rs (strip `/v1.NN`, reject non-canonical paths) until T014
  passes.
- [ ] T021 [US1] Implement the typed bodies in crates/depths-guard-core/src/body.rs:
  - `ContainerCreate` (`HostConfig`, `NetworkingConfig`, `Labels`, `Image`), `ContainerUpdate`,
    `ExecCreate`, `NetworkCreate`, `NetworkConnect`, `VolumeCreate`;
  - `#[serde(deny_unknown_fields)]` on `HostConfig`, with an allowlist of the known fields at the
    current Docker API version;
  - a 1 MiB cap.
- [ ] T022 [US1] Implement `judge(endpoint, body, facts, cfg) -> Verdict` in
  crates/depths-guard-core/src/rules.rs, using `depths_core::security` (`normalise_host_path`,
  `classify_host_path`, `normalise_cap`, `is_default_cap`, `unconfined_opt`).
  - Every form in research R3 is judged; any escape-level form refuses.
  - Missing facts give `NeedsUpstream`.
  - Run until T015–T017 pass.
- [ ] T023 [US1] Implement the base `GuardConfig` in crates/depths-guard-core/src/config.rs:
  `stacks_dir`, `upstream`, `listen`, `allow_build`, `runtimes`, `[roots] allow`, `[protected]`.
  - TOML with `deny_unknown_fields`.
  - Validation at load per data-model.md: roots are absolute, normalised, not sensitive, and
    existing (the existence check is passed in as a closure, so the crate stays I/O-free).
  - Tests for each load error.
- [ ] T024 [US1] Implement the path walk in apps/depths/guard/src/paths.rs: `openat2` with
  `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS` from the root's directory fd, through `rustix`, giving
  facts for `judge`. Run until T019 passes.
- [ ] T025 [US1] Implement the upstream lookups in apps/depths/guard/src/lookup.rs:
  - container inspect (stored config and labels) and volume inspect (driver options);
  - resolving protected container ids by label `org.fathom.depths.protected=true` and by name
    from `protected.containers`, at start and refreshed on miss;
  - any lookup failure refuses.
- [ ] T026 [US1] Implement the proxy in apps/depths/guard/src/proxy.rs and
  apps/depths/guard/src/serve.rs:
  - an HTTP/1.1 hyper server and client to `upstream`;
  - the body is buffered, judged, then forwarded unchanged;
  - `NeedsUpstream` loops through lookup/paths and judges again;
  - upgrades go through `hyper::upgrade` both ways after judging;
  - streaming responses pass through;
  - 403 and 502 bodies per contracts/guard.md.

  Run until T018 passes.
- [ ] T027 [US1] Implement the guard log in apps/depths/guard/src/log.rs: JSON lines in
  `DEPTHS_GUARD_LOG_DIR` and on stderr, rotated at 10 MiB, keeping 5 files. Fields per
  data-model.md GuardLogEntry; no request body is ever written.
- [ ] T028 [US1] Implement `depths-guard health` and the `/_depths_guard/v1/health` route in
  apps/depths/guard/src/health.rs and apps/depths/guard/src/main.rs.
  - `serve` exits non-zero on a bad config, with one line naming the key.
  - A debug-only `#[cfg(debug_assertions)]` request recording mode (`--record <file>`) writes each
    endpoint seen.
- [ ] T029 [US1] Record the endpoint fixtures, main session only. Run the guard in record mode in
  front of a socket proxy, then run compose `up`, `down`, `pull`, `logs` and `exec`, plus a helper
  run, against the example stacks. Commit the recordings to apps/depths/guard/tests/fixtures/, and
  add a test that every recorded endpoint is allowed.
- [ ] T030 [US1] Add the Dockerfile target `guard` in apps/depths/Dockerfile:
  - `depths-guard` is built statically in the existing cargo-chef builder;
  - `FROM scratch`, `COPY` the binary, `USER 65532:65532`, `ENTRYPOINT ["/depths-guard"]`.
- [ ] T031 [US1] Wire the topology in apps/depths/compose.yaml per contracts/guard.md:
  - a new `guard` service, hardened, with the label `org.fathom.depths.protected=true`, mounts
    `./guard.toml:ro`, the stacks dir `:ro` at the same path, and `./guard-log`, healthcheck
    `["CMD", "/depths-guard", "health"]`, and `restart: ${RESTART_POLICY:-no}`;
  - a new internal network `depths-guard`;
  - `depths` leaves `docker-api` and sets `DOCKER_HOST=tcp://guard:2375`;
  - `socket-proxy` gains the protected label.

  Check it with `docker compose config` (dockerops `config`, `mode: validate`). Ask before changing
  any running deployment.
- [ ] T032 [P] [US1] Add apps/depths/guard.toml.example (`stacks_dir`, defaults, `[protected]`, no
  exceptions, a commented example exception), and add `DEPTHS_GUARD_IMAGE` and `GUARD_LOG_DIR` to
  apps/depths/.env.example.
- [ ] T033 [US1] Server: attach guard refusals.
  - In apps/depths/server/src/ops.rs, on a failed op, run `guard_refusal` on stderr. Then:
    - set `OpFinished.guard`;
    - write `AuditAction::GuardRefused`;
    - map `guard_unreachable` to the `RuntimeGuardUnreachable` refusal.
  - Do the same in apps/depths/server/src/files.rs for helper starts (`FileRefused::Guard`).
  - Tests in apps/depths/server/tests/ws.rs use a scripted runtime-cli failure.
- [ ] T034 [P] [US1] Web: show the refusal next to the service it names, in plain words with the
  rule name, in apps/depths/web/src/features/stacks/OperationOutput.tsx and StackDetail.tsx, and
  show "the runtime guard is not answering" for `RuntimeGuardUnreachable`.
  - Unit test in apps/depths/web/tests/unit/ops.test.ts.
- [ ] T035 [US1] Fake backend: scripted `OpFinished { guard }` and `RuntimeGuardUnreachable` in
  apps/depths/web/tests/support/fake-backend.ts.
  - Playwright in apps/depths/web/tests/e2e/us8-guard/refusal.spec.ts: deploying a stack that
    binds `/etc` shows the refusal beside the service, and a guard-down deploy shows the
    unreachable message.

**Checkpoint**: US1 is shippable on its own (MVP). Deploying it needs T030–T032 and a `guard
check` (T043) on the target host first. Until T043 lands, review the target's stacks by hand.

---

## Phase 4: User Story 2 - A deliberate exception for a stack that needs one (Priority: P1)

**Goal**: exceptions only in host-owned `guard.toml`, narrow per project/service/setting, never
for the privileged flag or a runtime socket, and shown read-only in Depths.

**Independent Test**:

- Rule tests: an exception for `media/app` and `/dev/dri` lets exactly that through. `other` asking
  for `/dev/dri` is refused, and `media/app` asking for a second device is refused.
- Load tests: `privileged = true` and a socket bind both fail the load.
- Playwright `us8-guard`: the exceptions list.

### Tests for User Story 2 (write first)

- [ ] T036 [P] [US2] Exception load tests in crates/depths-guard-core/tests/config.rs. Each of
  these is a load error:
  - an unknown key, including `privileged = true`;
  - empty `project` or `service`;
  - no grants;
  - a device not under `/dev/`;
  - a device or bind that is a runtime socket;
  - a writable sensitive bind;
  - cap `ALL`;
  - a namespace other than network/pid/ipc;
  - a duplicate (project, service, image).

  A full valid example (data-model.md) loads.
- [ ] T037 [P] [US2] Exception matching tests in crates/depths-guard-core/tests/rules.rs:
  - the exact project/service/image match allows exactly the listed grants, and the verdict is
    `Allow { exception: Some(..) }`;
  - another project is refused;
  - the same service asking for one more device, cap or bind is refused;
  - an image mismatch is refused when `image` is set;
  - a read-only grant does not allow a writable bind;
  - `Privileged` is refused even with an exception present.

### Implementation for User Story 2

- [ ] T038 [US2] Implement `[[exception]]` parsing and validation in
  crates/depths-guard-core/src/config.rs. Fields `project`, `service`, `image?`, `devices`,
  `binds{path, read_only}`, `caps`, `namespaces`, `note?` and `granted?`, with the load rules in
  T036.
- [ ] T039 [US2] Implement matching in crates/depths-guard-core/src/exception.rs. Wire it into
  `judge` in rules.rs, so each escape-level setting is checked against the matched exception's
  grants one at a time. Run until T037 passes.
- [ ] T040 [US2] Guard: log `outcome: "exception"` with the exception reference in
  apps/depths/guard/src/log.rs.
  - Serve `GET /_depths_guard/v1/exceptions[?project=]` in apps/depths/guard/src/serve.rs.
  - Tests in apps/depths/guard/tests/proxy.rs: logs the exception use; the endpoint filters by
    project; there are no write methods (405 or 404).
- [ ] T041 [US2] Guard: `SIGHUP` reload in apps/depths/guard/src/serve.rs. A bad file keeps the old
  config and logs the error.
  - Test: reload with a valid file applies it, and reload with an invalid one keeps the previous
    one.
- [ ] T042 [US2] Server: `GetGuardExceptions { stack }` in apps/depths/server/src/ops.rs. A
  read-only GET to `DEPTHS_GUARD_URL` maps to `GuardExceptions`, and an empty list when there are
  none. An unreachable guard gives `RuntimeGuardUnreachable`.
  - Tests in apps/depths/server/tests/ws.rs use a tiny fake guard listener.
- [ ] T043 [US2] Implement `depths-guard check [--config]` in apps/depths/guard/src/check.rs.
  - List containers through the upstream, and judge each stored config as a start.
  - For each refused container, print the project, service and rules, plus a suggested
    `[[exception]]` block with `image` filled in.
  - Exit 1 if any would be refused (FR-023).
  - Test against the fake upstream in apps/depths/guard/tests/proxy.rs.
- [ ] T044 [P] [US2] Web: an exceptions section on the stack page, in
  apps/depths/web/src/features/stacks/GuardExceptions.tsx, rendered from StackDetail.tsx.
  - Each grant shows its kind, value and read-only flag, plus the note and "since" date.
  - Read-only, with no edit control. A line says exceptions are granted on the host.
  - Uses the mobile-first list pattern.
- [ ] T045 [US2] Fake backend `GuardExceptions` in apps/depths/web/tests/support/fake-backend.ts.
  - Playwright in apps/depths/web/tests/e2e/us8-guard/exceptions.spec.ts: the list renders, an
    empty state shows, and no control can change them.

**Checkpoint**: US1 + US2 is the full guard. `guard check` is available for the steam upgrade.

---

## Phase 5: User Story 3 - The volume helper can only touch the files it was given (Priority: P2)

**Goal**: the helper runs from a `FROM scratch` image holding only its binary, identified by build
id, with no fallback to the main image.

**Independent Test**:

- `cargo test -p depths-runtime-cli` (argv) and `cargo test -p depths-server --test files` (build
  mismatch).
- The opt-in T080: the image lists one file, and every 053 file test passes against it.

### Tests for User Story 3 (write first)

- [ ] T046 [P] [US3] Argv test in crates/depths-runtime-cli/src/helper.rs: `--entrypoint
  /depths-volume-helper`. Every 053 flag is still present (`--network none`, `--read-only`,
  `--cap-drop ALL`, `--security-opt no-new-privileges`, `--memory 256m`, `--pids-limit 32`), and
  there is no `--cap-add`.
- [ ] T047 [P] [US3] Build-check tests in apps/depths/server/tests/files.rs:
  - a helper `Hello` with a different `build` gives `helper_unavailable { reason: build_mismatch }`
    and starts no op;
  - an image without the matching `org.fathom.depths.helper.build` label gives
    `helper_unavailable`;
  - no code path falls back to the server's own image.
- [ ] T048 [P] [US3] Guard test in crates/depths-guard-core/tests/rules.rs: the exact create body
  `helper_argv` produces (fixture) is allowed with no exception (US3 scenario 3).

### Implementation for User Story 3

- [ ] T049 [US3] `Hello` gains `build: String` in crates/depths-volumes/src/op.rs (keep `PROTO = 1`,
  since the field is additive), and apps/depths/volume-helper/src/main.rs sends
  `depths_volumes::BUILD_ID`.
- [ ] T050 [US3] Server: in apps/depths/server/src/files.rs (and the runtime-cli helper adapter):
  - at start, resolve `DEPTHS_HELPER_IMAGE` to an image id and check the build label;
  - check `Hello.build == BUILD_ID`;
  - otherwise files are `helper_unavailable` with the reason.

  Change the entrypoint in crates/depths-runtime-cli/src/helper.rs. Run until T046 and T047 pass.
- [ ] T051 [US3] Change apps/depths/Dockerfile:
  - The builder computes `sha256sum` of the static helper, exports it as `DEPTHS_BUILD_ID`, and
    builds both the helper and the server with it. Build the helper first, then pass the id to the
    server build in the same stage.
  - New target `helper`: `FROM scratch`, `COPY` only `/depths-volume-helper`,
    `LABEL org.fathom.depths.helper.build=$DEPTHS_BUILD_ID`.
  - Remove the helper from the main runtime image.
- [ ] T052 [US3] Compose and env changes:
  - In apps/depths/compose.yaml, set `DEPTHS_HELPER_IMAGE=${DEPTHS_HELPER_IMAGE:-depths-volume-helper:dev}`
    for `depths`, and add the image to the build.
  - In apps/depths/.env.example, document that the helper image is required.
- [ ] T053 [US3] Update apps/depths/server/tests/files_docker.rs: seed volumes with the main image,
  which still has `/bin/sh`, and run the helper from `DEPTHS_HELPER_IMAGE`. Do not run it here; the
  run is T080.

**Checkpoint**: files work with the new image. The main image no longer carries the helper.

---

## Phase 6: User Story 4 - Administrators prove it is them with a passkey (Priority: P2)

**Goal**: passkey enrolment and verification. Admin and owner sessions stay restricted until they
verify. Owners recover through a host-only code.

**Independent Test**:

- `cargo test -p depths-server --test passkeys` with the soft passkey:
  - enrol;
  - verify;
  - replay and wrong-origin rejected;
  - an unverified admin gets the restricted view;
  - a copied cookie in a fresh session cannot act as admin (SC-005);
  - recovery.
- Playwright `us7-passkeys` with the virtual authenticator.

### Tests for User Story 4 (write first)

- [ ] T054 [P] [US4] Store tests in apps/depths/server/src/passkeys.rs:
  - atomic write (temp, fsync, rename);
  - a corrupt file is moved aside (not deleted) and Depths starts empty, with this audited;
  - `name` is 1–64 characters, trimmed, and unique per user;
  - the last passkey cannot be removed except by recovery.
- [ ] T055 [P] [US4] Ceremony tests in apps/depths/server/tests/passkeys.rs, using
  `webauthn-authenticator-rs` softpasskey:
  - register, then verify, sets `verified_at`;
  - a replayed assertion gives `PasskeyRefused { reason: replayed }`;
  - an assertion for another origin gives `wrong_origin`;
  - a ceremony id from another session gives `expired`/refused;
  - a ceremony older than 2 minutes gives `expired`;
  - a ceremony is single-use;
  - every outcome is audited.
- [ ] T056 [P] [US4] Verification gate tests in apps/depths/server/tests/passkeys.rs:
  - an admin with no passkey gets `PasskeyEnrolmentRequired` for admin actions and can only enrol;
  - an unverified admin with a passkey gets the restricted access set (the 052 path);
  - a verified admin gets admin access;
  - an admin on an untrusted network still gets only the restricted view after verifying
    (FR-022);
  - a second session (another browser) of the same admin is unverified;
  - a user promoted to admin must enrol before their next admin action.
- [ ] T057 [P] [US4] Recovery tests in apps/depths/server/tests/passkeys.rs:
  - the CLI writes only the SHA-256 and prints the code once;
  - `RecoverPasskeys` with the code removes that person's passkeys and allows one `Register`
    without an assertion;
  - a wrong code, an expired one (15 min) or a reused one gives `recovery_invalid`;
  - a new code replaces the old one;
  - both steps are audited, with no code in the audit.
- [ ] T058 [P] [US4] Web codec tests in apps/depths/web/tests/unit/passkeys.test.ts: base64url
  round trips for empty, 1–3 byte tails and 64 KiB, padding refused, and the options and
  credential conversion to and from the protocol JSON.

### Implementation for User Story 4

- [ ] T059 [US4] Implement the passkey store in apps/depths/server/src/passkeys.rs:
  `<data_dir>/passkeys.json` `{ version: 1, users: { <user>: [ { id, name, created, last_used,
  credential } ] } }`, written in the `users.rs` atomic pattern, loaded at start. Run until T054
  passes.
- [ ] T060 [US4] Implement ceremonies in apps/depths/server/src/passkeys.rs:
  - the `Webauthn` builder from `public_url` (RP id = host, origin = origin; `localhost` for
    loopback);
  - an in-memory ceremony table `{ id, session_handle, kind, state, expires (2 min) }` with sweep;
  - `begin_register`, `finish_register`, `begin_verify`, `finish_verify`;
  - `last_used` and counter update.
- [ ] T061 [US4] `SessionMeta` gains `verified_at: Option<DateTime<Utc>>` in
  apps/depths/server/src/session_meta.rs. It is set on a successful Verify or StepUp, and dropped
  with the session.
- [ ] T062 [US4] Wire the commands in apps/depths/server/src/ops.rs and apps/depths/server/src/ws.rs:
  - `GetVerification`;
  - `PasskeyBegin`/`PasskeyFinish`;
  - `ListPasskeys` (own only);
  - `PreviewPasskeyRemove` (refused for the last passkey), with the removal applied on `Confirm`;
  - push `Verification` after sign-in and after any change.

  Run until T055 passes.
- [ ] T063 [US4] Add the verification gate in apps/depths/server/src/step_up.rs, called from
  ops.rs after the 052 trust gate.
  - An admin or owner without `verified_at` is handled as `Role::User` with no stacks.
  - A person with no passkey gets `PasskeyEnrolmentRequired` for admin commands.

  Run until T056 passes.
- [ ] T064 [US4] Recovery:
  - Add the subcommand `depths passkey-recovery --user <id>` in apps/depths/server/src/main.rs
    (parse argv before `run()`; it touches only the data dir and needs no runtime). It writes
    `<data_dir>/passkey-recovery.json` `{ user, code_sha256, expires, used }` and audits
    `PasskeyRecoveryIssued`.
  - Add `RecoverPasskeys { code }` in ops.rs.

  Run until T057 passes.
- [ ] T065 [P] [US4] Web passkey module in apps/depths/web/src/features/passkeys/:
  - `codec.ts` (base64url and option/credential conversion);
  - `api.ts` (`navigator.credentials.create`/`get`; cancellation maps to `cancelled`);
  - `store.ts` (verification state from `Verification` events).

  Run until T058 passes.
- [ ] T066 [US4] Web enrol and verify gate in apps/depths/web/src/features/passkeys/PasskeyGate.tsx,
  mounted in the app shell.
  - An admin or owner without a passkey is asked to register, with an admin-actions-locked banner.
  - With a passkey, a "Confirm it's you" prompt appears after sign-in.
  - If they cancel, they get the restricted view with a retry.
- [ ] T067 [US4] Web account passkeys panel in
  apps/depths/web/src/features/passkeys/PasskeysPanel.tsx, rendered on SessionsPage.tsx:
  - a list by name, created and last used;
  - add with a name;
  - remove through the preview dialog (disabled for the last one);
  - an "Enter recovery code" form.
- [ ] T068 [US4] Fake backend passkey commands in apps/depths/web/tests/support/fake-backend.ts. It
  accepts any well-formed credential, and is scriptable for refusals.
  - Playwright in apps/depths/web/tests/e2e/us7-passkeys/enrol.spec.ts and
    apps/depths/web/tests/e2e/us7-passkeys/manage.spec.ts, using the CDP
    `WebAuthn.addVirtualAuthenticator`. Cover:
    - forced enrolment;
    - verify after sign-in;
    - cancel gives the restricted view;
    - add a second passkey and remove one;
    - the last passkey cannot be removed;
    - recovery code flow;
    - phone width.

**Checkpoint**: stolen provider accounts and cookies no longer give admin.

---

## Phase 7: User Story 5 - Risky actions ask again (Priority: P3)

**Goal**: every role gives a fresh passkey assertion for the FR-017 actions, folded into the
confirm step, with one check covering the window.

**Independent Test**:

- `cargo test -p depths-server --test passkeys` (step-up section): each risky command past the
  window refuses until there is an assertion, within the window it passes, and a failure changes
  nothing.
- Playwright `us7-passkeys/step-up.spec.ts`.

### Tests for User Story 5 (write first)

- [ ] T069 [P] [US5] Step-up tests in apps/depths/server/tests/passkeys.rs. With `verified_at`
  older than the window:
  - `Preview` of a file write carries `step_up`;
  - `Confirm` without `assertion` gives `StepUpRequired` and changes nothing;
  - `Confirm` with a valid assertion applies and refreshes `verified_at`;
  - a second risky `Confirm` inside the window needs none;
  - `OpenShell` without an assertion gives `StepUpRequired { request }`, and the retry with the
    assertion opens it;
  - a failed assertion is audited and changes nothing (FR-019);
  - a deploy without escape findings needs no step-up;
  - a deploy with them does;
  - `PutUser`, `RemoveUser` and whitelist changes need step-up;
  - a plain user with no passkey gets `PasskeyEnrolmentRequired` for a risky action (Q2 = A).
- [ ] T070 [P] [US5] Window tests in apps/depths/server/tests/passkeys.rs: `DEPTHS_STEP_UP_WINDOW=30`
  is honoured, using an injected clock rather than sleeps, so the test is deterministic.

### Implementation for User Story 5

- [ ] T071 [US5] Implement the step-up gate in apps/depths/server/src/step_up.rs, using
  `depths_core::role::step_up_needed`.
  - Fresh means within the window.
  - Otherwise verify the command's `assertion` against the ceremony issued with the preview or the
    `StepUpRequired`.
  - Audit `StepUpRequired`, `PasskeyVerified` and `PasskeyFailed`.

  Call it in apps/depths/server/src/ops.rs after role and stack access, and make `Preview` set
  `step_up` when confirming will need it.
- [ ] T072 [US5] Apply step-up to `OpenShell` in apps/depths/server/src/ws/streams.rs: check before
  `docker exec` is spawned, and on a stale session reply `StepUpRequired { request }`.
  - Tests in apps/depths/server/tests/ws.rs.
- [ ] T073 [US5] Web confirm-step integration in
  apps/depths/web/src/features/stacks/PreviewDialog.tsx (and the files and users dialogs that reuse
  it):
  - when `preview.step_up` is set, pressing Confirm calls `navigator.credentials.get` and sends
    `Confirm { assertion }`;
  - cancelling keeps the dialog open with "Not confirmed, nothing changed".

  Unit tests in apps/depths/web/tests/unit/ops.test.ts.
- [ ] T074 [US5] Web shell retry in apps/depths/web/src/features/tabs/ (shell open path): on
  `StepUpRequired`, prompt, then resend `OpenShell { assertion }`. Prompt for enrolment on
  `PasskeyEnrolmentRequired`.
- [ ] T075 [US5] Fake backend step-up scripting in apps/depths/web/tests/support/fake-backend.ts.
  - Playwright in apps/depths/web/tests/e2e/us7-passkeys/step-up.spec.ts:
    - a stale session asks inside the confirm dialog and then saves;
    - a second save within the window does not ask;
    - cancel changes nothing;
    - shell open asks first;
    - a plain user is sent to enrol.

**Checkpoint**: all five stories work.

---

## Phase 8: Polish & Cross-Cutting

- [ ] T076 [P] Write docs/guides/depths/guard.md:
  - what the guard is and where it sits;
  - what it refuses (the rule list, in plain words);
  - `allow_build` and why it weakens the boundary;
  - exceptions: format, narrowness, and that labels can be forged so the image match matters;
  - `guard check` before upgrading;
  - the guard log;
  - the `nosymfollow` recommendation;
  - the residual risk (FR-023);
  - the upgrade notes (stopped escape-level containers are not restarted through Depths; steam
    order).
- [ ] T077 [P] Write docs/guides/depths/passkeys.md: registering, confirming, step-up and the window,
  managing passkeys, the recovery code via `docker compose exec depths depths passkey-recovery`,
  enrolling right after upgrade, and that changing `DEPTHS_PUBLIC_URL`'s host invalidates
  passkeys.
- [ ] T078 [P] Update docs/guides/depths/security.md (threat model, what the guard narrows, helper
  image, passkeys and step-up, the new audit actions), docs/guides/depths/install.md (guard
  service, `guard.toml`, `guard-log`, helper image, kernel ≥ 5.6), and docs/guides/depths/README.md
  links. Cross-reference contracts/config.md from
  specs/050-depths-stack-manager/contracts/config.md.
- [ ] T079 Main session only, opt-in: write and run apps/depths/guard/tests/docker.rs
  (`DEPTHS_IT_DOCKER=1`) per quickstart.md §2.
  - Set up scratch `depths-it-<random>` networks, a socket proxy and the guard image.
  - Check that every FR-003 case gives `depths-guard refused`, the example stacks deploy, a
    planted symlink is refused, `--volumes-from` the proxy is refused, `docker-api` attach is
    refused, and one exception passes one device.
  - Clean up by name.
- [ ] T080 Main session only, opt-in: build the `helper` target, check it holds one file (SC-004),
  and run apps/depths/server/tests/files_docker.rs with `DEPTHS_HELPER_IMAGE=depths-volume-helper:dev`
  (quickstart.md §3). Prune build cache afterwards (`docker builder prune --filter until=24h`).
- [ ] T081 Run every quickstart.md §1 command one at a time, and fix what fails.
- [ ] T082 Run `scripts/hygiene.sh` and remove anything this work left dangling.
- [ ] T083 Main session only: one full `scripts/ci.sh` on the combined branch.
- [ ] T084 Brochure and marketing screenshots of the passkey prompt, account passkeys panel and
  stack exceptions list, last.

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (T001–T005)**: no dependencies. T003–T005 run in parallel with T001 and T002.
- **Foundational (T006–T013)**: after Setup, and blocks every story.
  - T006 → T007 → T008.
  - T009–T013 run in parallel, with T009 after T006.
- **US1 (T014–T035)**: after Foundational.
  - The tests T014–T019 run in parallel.
  - Then T020 → T021 → T022 and T023, while T024, T025 and T027 run in parallel.
  - T026 comes after T022–T025.
  - T028 → T029 (main session).
  - T030–T032 come after T026.
  - The server and web half, T033 → T034/T035, needs only T012, so it can start early.
- **US2 (T036–T045)**: after T022/T023 (rules and config) and T026 (proxy).
- **US3 (T046–T053)**: after Foundational (T013). It is independent of US1 and US2, except T048,
  which needs T022.
- **US4 (T054–T068)**: after Foundational (T003, T006–T009). It is independent of US1–US3.
- **US5 (T069–T075)**: after US4 (T060–T063) and T011.
- **Polish**: after the stories each task documents. T079–T084 run strictly last, in order (T081
  before T083).

### Story completion order

```
Setup → Foundational ─┬→ US1 (MVP) → US2 ─────────┐
                      ├→ US3 ─────────────────────┤
                      └→ US4 → US5 ───────────────┴→ Polish → docker ITs → full CI → screenshots
```

### Within each story

Tests first, and failing → core crate → adapter or server → protocol wiring → UI → the story's
Playwright green → commit.

---

## Parallel Execution Examples

### After Foundational (three agents, `CARGO_BUILD_JOBS=4`, no CI, targeted tests only)

```text
Agent A (guard):     T014–T019 tests, then T020–T028, then T036–T041, T043
Agent B (helper):    T046–T053
Agent C (passkeys):  T054–T068, then T069–T075
Main session:        T001–T013, T029 (recording), T030–T032 compose/Dockerfile, T033–T035, T042,
                     T044–T045, merges
```

All three agents edit apps/depths/Dockerfile or compose.yaml at some point. Keep those edits in the
main session (T030, T031, T051, T052) to avoid conflicts. Agent B leaves T051 and T052 as notes for
the main session.

### User Story 1

```text
T014, T015, T016, T017, T018, T019 together (tests)
T024 (paths), T025 (lookup), T027 (log) together, while T020 → T022 runs in the core crate
```

### User Story 4

```text
T054, T055, T056, T057, T058 together (tests)
T065 (web module) alongside T059 → T063 (server)
```

---

## Implementation Strategy

### MVP first (US1)

1. Setup and Foundational.
2. US1. Stop when the rule table and the proxy tests are green and the fixtures are recorded.
3. Do not deploy to steam until US2's `guard check` (T043) has listed what needs exceptions there.
   Ask before deploying.

### Incremental delivery

1. US1 and US2: the host boundary. This is a deploy candidate after `guard check`, and needs
   asking first.
2. US3: the helper image. It can ship alone at any point.
3. US4: the second factor. Owners and admins enrol right after deploying.
4. US5: step-up.
5. Polish, the opt-in docker tests, one full CI, and screenshots last.

### Notes

- The guard never rewrites a request. A task that seems to need it is a design error; check
  research R1 and R3.
- Any change to the helper argv updates contracts/helper.md (053), the T046 argv test, and the T048
  guard fixture together.
- A new Docker API field that the guard refuses as `unknown_field` gets allowlisted only together
  with a rule row that judges it.

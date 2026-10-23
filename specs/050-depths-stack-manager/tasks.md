# Tasks: Depths, a self-hosted Compose stack manager

**Input**: Design documents from `/specs/050-depths-stack-manager/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/protocol.md](./contracts/protocol.md),
[contracts/config.md](./contracts/config.md), [contracts/shared-crates.md](./contracts/shared-crates.md),
[quickstart.md](./quickstart.md)

**Tests**: REQUIRED. Constitution Principle III is non-negotiable. Each user story needs Playwright
E2E tests against the scripted fake backend. Each protocol variant needs a round-trip test. Each
rule needs a test that fails without it, written first.

**Organization**: Setup and Foundational cover the constitution gate, the workspace and the
extraction (plan phases A and B). The user-story phases cover Depths itself (plan phases C and D).
The final phase covers shipping (plan phase E).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel: different files, and no dependency on an incomplete task.
- **[Story]**: US1–US7, used in the user-story phases only.
- **[B1]/[B2]/[B3]**: extraction sub-track in Phase 2 (not a story); each sub-track is one parallel agent.
- T087 was added after analysis; it belongs to US1 and runs before T048.

## Rules for every task in this file

- **Fixtures**: one image, `busybox`. Test stacks live in temp dirs, with project names prefixed
  `depths-test-`.
- **Docker**: remove what a test created with `docker compose down`, by name. Never prune, never
  remove an image, never touch a container or volume the test did not create.
- **No kind clusters.**
- **Builds**: `CARGO_BUILD_JOBS=4`, and `cargo check -p <crate>` while iterating. Playwright sizes
  its own workers.
- **Fathom must stay green at every commit** (FR-044). An extraction task is done only when all of
  these hold:
  - `scripts/ci.sh` passes.
  - `cargo test -p fathom --test export_bindings` leaves `ui/src/protocol/generated/` unchanged
    (`git diff --exit-code`).
  - The Fathom E2E projects touching containers, terminal and theme pass.
- **Commits**: one signed commit per task or tight group of tasks.

---

## Phase 1: Setup (plan phase A)

- [X] T001 Amend the constitution to 1.2.0 via `/speckit-constitution` in `.specify/memory/constitution.md` (MINOR, with a Sync Impact Report). The amendment must:
  - Apply the principles to every app in the suite.
  - Generalise Principle II to "each app owns one versioned, Rust-defined contract; shared domains define wire types once in a shared crate that each app's contract re-exports".
  - Add a "Depths server (Linux container)" tier: Docker host, sign-in required, socket access documented as root-equivalent, socket proxy supported.
  - Add CodeMirror 6, `notify` and the optional OpenTelemetry exporter to Stack.
  - Update the dependent templates the report lists.
- [X] T002 Create the root pnpm workspace:
  - `pnpm-workspace.yaml` with members `ui`, `packages/*` and `apps/depths/web`.
  - Root `package.json`: private, with `packageManager` copied from `ui/package.json`.
  - Move `ui/pnpm-lock.yaml` to `/pnpm-lock.yaml` and regenerate it with `pnpm install`.
  - Keep `ui/package.json`'s build script working when deps are hoisted. It calls `node node_modules/typescript7/bin/tsc -b`; switch it to `pnpm exec tsc7` or the bin name if hoisting moves it.
- [X] T003 Update every hard-coded `ui/` install and lockfile path to the root workspace so each runs `pnpm install --frozen-lockfile` at the root and `pnpm --filter fathom-ui …` for builds:
  - `pipeline/src/pipeline.rs:129-228`
  - `scripts/ci.sh` (lines ~40, 48, 94-107)
  - `scripts/build-windows.sh` (`--workdir`, and corepack resolving from the root `packageManager`)
  - `scripts/third-party-licenses.sh`
  - `.github/workflows/release.yml:91-110`
  - `deploy/Dockerfile:2-9,47`
  - `Makefile` (`UI :=`)
  - `Makefile.toml:112`
  - `k8s/cluster.sh:115`

  Then verify with `scripts/ci.sh` and `make build-linux` (or the pipeline's UI step).
- [X] T004 Add workspace members and empty crates, each with a `lib.rs` doc comment, `edition = "2024"` and `license` matching the root:
  - `crates/container-runtime`
  - `crates/web-auth`
  - `crates/audit-log`
  - `crates/depths-core`
  - `crates/depths-stacks-fs`
  - `crates/depths-protocol`
  - `apps/depths/server` (bin `depths`)

  Add them to `[workspace] members` in `Cargo.toml`, and check that `cargo check --workspace` passes.
- [X] T005 [P] Create empty packages:
  - `packages/theme` (`@companion/theme`), `packages/ui` (`@companion/ui`) and `packages/terminal` (`@companion/terminal`), each with `package.json` (private, `type: module`, `exports`), `tsconfig.json` extending a new root `tsconfig.base.json`, and `src/index.ts`.
  - `apps/depths/web` (`@companion/depths-web`) from Vite's React-TS template trimmed to the repo's ESLint and Vitest setup, with the `@/` alias.
- [X] T006 [P] Extend `scripts/hygiene.sh` to report leftover `depths-test-*` compose projects and temp stacks dirs. Extend `Makefile`:
  - `depths-dev`: server plus `vite dev`.
  - `depths-image`: depends on `gc`.
  - `depths-test`.

---

## Phase 2: Foundational: extraction (plan phase B; blocks every user story)

Three sub-tracks, B1, B2 and B3, touch disjoint files. Run them as parallel agents in separate
worktrees and merge each green.

### B1. `crates/container-runtime` (contract: shared-crates.md)

- [X] T007 [B1] Move `Runner`, `CmdOutput`, `RunError` and `SystemRunner` from `src/backend/kind.rs:33-67` to `crates/container-runtime/src/runner.rs`. Add `run_with_stdin(program, args, cwd, stdin)`, whose default impl returns `RunError::Unsupported`, and implement it on `SystemRunner`. `src/backend/kind.rs` re-imports these types (`pub use container_runtime::runner::*`). Then make `fathom` depend on the crate.
- [ ] T008 [B1] Write the failing test first: add a test in `crates/container-runtime/src/sink.rs` that a `StreamSink` receiving `emit` gets the `StreamEvent`s produced by `logs.rs` from a scripted process. Then define `trait StreamSink { fn emit(&self, StreamEvent) -> bool }` and move `StreamEvent` / `StreamId` (from `src/protocol/v1/event.rs`) into `crates/container-runtime/src/wire.rs`, with their serde and ts-rs derives unchanged.
- [ ] T009 [B1] Move the 37 types in `src/protocol/v1/container_runtime.rs` and `KindRuntimeKind` (`src/protocol/v1/kind_cluster.rs:19`) to `crates/container-runtime/src/wire.rs`. Replace the originals with `pub use container_runtime::wire::*;` and keep the ts-rs `export_to` paths so the bindings land in `ui/src/protocol/generated/` unchanged. Verify with `git diff --exit-code ui/src/protocol/generated`.
- [ ] T010 [B1] Move `stream.rs`, `probe.rs`, `compose.rs`, `stats.rs`, `inventory.rs`, `grouping.rs`, `mutate.rs`, `blast_radius.rs`, `preview.rs`, `logs.rs` and `session.rs`, plus `Cli` and `parse_size` from `mod.rs`, from `src/backend/container_runtime/` into `crates/container-runtime/src/`. Then cut these seams:
  - `logs.rs` and `session.rs` write to `&dyn StreamSink`.
  - The kind-label parse in `inventory.rs:28` becomes a Fathom post-pass in `src/backend/container_runtime/mod.rs`.
  - `PreviewContext` gains `app_name: &str` and `cluster: Option<&dyn ClusterImpact>`, replacing the "Fathom" strings at `preview.rs:73-90` and the `breaks_clusters` computation.
  - `cluster_link.rs` stays in Fathom and implements `ClusterImpact`.
- [ ] T011 [B1] Split `adoption.rs`:
  - The crate keeps `AdoptionStore::open(path)` and `requirement_for_scope`.
  - Fathom keeps `requirement(&BackendCmdV1)` and opens the store at `fathom_config_dir().join("container_adoption.json")`. The path is unchanged, per Principle VI.
  - Replace `set_thread_state_root` in the adoption tests with temp-dir paths.
- [ ] T012 [B1] Move `ScriptedProcess` and the mock runners behind a `test-util` feature in `crates/container-runtime/src/test_util.rs`, and enable it from `fathom`'s `[dev-dependencies]`. Fathom keeps a `StreamSink` impl over `BackendEventEnvelope` (in `src/backend/log_streams.rs`) and its dispatch in `src/backend/cmd/container_runtime.rs`, untouched except for imports.
- [ ] T013 [B1] Gate: check that `cargo test -p container-runtime` and `cargo test -p fathom` pass, that bindings drift is zero, and that `scripts/ci.sh` is green. Check that `cargo tree -p container-runtime` has no `kube`, `tauri` or `fathom`, and add that check to `scripts/ci.sh`.

### B2. `crates/web-auth` and `crates/audit-log`

- [ ] T014 [P] [B2] Move `server/src/session.rs` and `server/src/csrf.rs` to `crates/web-auth/src/{session,csrf}.rs` with their tests. `server/src/{session,csrf}.rs` become `pub use web_auth::…`. Extend `Identity` with `provider` (`oidc | github | discord`, defaulting to `oidc` so server behaviour is unchanged).
- [ ] T015 [B2] Move `server/src/oidc.rs` to `crates/web-auth/src/oidc.rs`:
  - Narrow the constructor from `HostedConfig` to `OidcConfig{issuer, client_id, client_secret, redirect_url, scopes}` + `ClaimsConfig{groups_claim, username_claim, groups_required}`.
  - The server builds both from `HostedConfig` and `AccessModel` in `server/src/main.rs` or wherever `OidcClient::discover` is called today.
  - Keep `map_identity` and its tests in the crate.
- [ ] T016 [B2] Write the failing test first: add an `IdentityProvider` conformance test in `crates/web-auth/tests/provider.rs`, run against an in-process fake OIDC issuer (axum + `rsa` dev-dependency, as the server tests already use). Then define the `IdentityProvider` trait in `crates/web-auth/src/provider.rs` and implement it for `OidcProvider`.
- [ ] T017 [B2] Lift `LoginStore` (`server/src/http.rs:31`), the cookie helpers and `session_from_headers` (`http.rs:121`) into `crates/web-auth/src/{login,cookie,extract}.rs`:
  - The cookie name comes from `CookieConfig` (Fathom: `__Host-fathom_session`).
  - Add an axum `SessionUser` extractor and a CSRF `tower::Layer` over `check_mutation`.
  - The server uses them, and its HTTP tests still pass unchanged.
- [ ] T018 [P] [B2] Write the failing tests first in `crates/audit-log/src/lib.rs`:
  - Append is a single write, and the file is opened `O_APPEND`.
  - The directory is 0700 and the file 0600.
  - `Retention::RotateAtBytes` renames to `audit-log.<n>.jsonl` and never rewrites an entry.
  - `CompactAfter(10_000)` keeps the server's current behaviour.

  Then move the writer and reader from `server/src/audit.rs`, generic over `E: Serialize + DeserializeOwned`.
- [ ] T019 [B2] Make `server/src/audit.rs` a wrapper implementing `fathom::backend::audit_log::AuditSink` over `AuditLog<AuditLogEntry>` with `CompactAfter(10_000)`, keeping the same path (`<state_dir>/audit/audit-log.jsonl`) and format.
- [ ] T020 [B2] Gate: check that `cargo test -p web-auth -p audit-log -p fathom-server` and the hosted E2E projects pass. Check that `cargo tree -p web-auth -p audit-log` has no `kube` or `fathom`, and add that check to `scripts/ci.sh`.

### B3. `@companion/theme`, `@companion/ui`, `@companion/terminal`

- [X] T021 [P] [B3] Move `ui/src/theme/{tokens.css,effects.css,apply.ts,color.ts,fonts.ts,BrandMark.tsx,CircuitTrace.tsx}` and the `cn` re-export from `ui/src/lib/utils.ts` into `packages/theme/src/`:
  - `applyTheme` takes a package-declared structural `ThemePrefs {mode, accent, effects, glitch}`.
  - `BrandMark` takes `name`/`glyph` props.
  - Fathom imports from `@companion/theme`, and `ui/src/index.css` imports the package CSS.
  - Move `ui/tests/unit/contrast.test.ts` to `packages/theme/tests/`, and add `packages/theme/tests` to the Vitest `include` that Fathom's unit run uses.
- [X] T022 [P] [B3] Move the 22 primitives in `ui/src/components/ui/` to `packages/ui/src/` with relative imports. `sonner.tsx` takes `theme` as a prop, and Fathom passes its store's mode. Add `@source "../../packages/ui/src"` (path relative to `ui/src/index.css`) so Tailwind 4 scans the package. Fathom imports `@companion/ui`, and `components.json` points shadcn at the package.
- [X] T023 [B3] Write the failing tests first in `packages/terminal/tests/`: Vitest tests for `RingBuffer` (cap 10,000, dropped-count marker) and for `LogView` (renders only the visible window of 10,000 lines, keeps the scroll anchor when following, shows "N older lines dropped").
- [X] T024 [B3] Implement `packages/terminal/src/`, then rewrite Fathom's `ui/src/components/panels/TerminalView.tsx` and `LogScroller.tsx` as thin store adapters onto these. Fathom's terminal and logs E2E must pass unchanged. The package contents:
  - `RingBuffer` (moved from `ui/src/store/ring-buffer.ts`).
  - `LogView`, virtualised with `@tanstack/react-virtual`.
  - `TerminalSurface({transport, subscribe, onResize})`, the store-free core of `ui/src/components/panels/TerminalView.tsx`.
  - `PaneStates` (from `components/shell/PaneStates.tsx`).
  - `YamlDiffView`.
- [X] T025 [B3] Gate: check that `pnpm -r test`, Fathom's lint and type check, and the Fathom E2E projects for theme, containers and terminal all pass. Check that a visual spot-check of 3 Fathom screens against `docs/guides` screenshots shows no change.

**Checkpoint**: shared code extracted, Fathom unchanged. Depths work can start.

---

## Phase 3: Depths backend foundation (plan phase C; blocks US1–US6)

- [ ] T026 [P] Write the failing tests first in `crates/depths-core/src/name.rs`: `valid_name` accepts `web`, `a_b-1`, `0x`, and rejects `Web`, `-a`, `_a`, `a.b`, `a b`, `""`, `..`. The rule to quote: "matches `^[a-z0-9][a-z0-9_-]*$`". Then implement it.
- [ ] T027 [P] Write the failing tests first in `crates/depths-core/src/discovery.rs`: compose file precedence is `compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml` ("the first present in Compose's order"), and `shadowed_files` lists the others. Then implement it over a `DirListing` input (no I/O).
- [ ] T028 [P] Write the failing tests first in `crates/depths-core/src/status.rs` with canned 048 `Container`s:
  - Each status: `not_deployed`, `running`, `partial`, `stopped`, `invalid`.
  - The rule "exited with code 0 whose restart policy is `no` counts as completed and never makes a stack partial".
  - `restarting` counts as not running.
  - The `unhealthy` flag.
  - `running_services`/`total_services`.

  Then implement it.
- [ ] T029 [P] Write the failing tests first in `crates/depths-core/src/role.rs`:
  - `pending` permits nothing; a `user` permits nothing on a stack outside their list.
  - The permission table in data-model.md, row by row, for `user` on an assigned stack, `user` on an unassigned stack, and `admin`.
  - Admin-only actions (create, delete, `down` with volumes or images, foreign and standalone views, audit, users) are refused to every `user`.

  Then implement `Role { Pending, User, Admin }`, `Access { role, stacks }` and `permits(access, stack, action)`, with no I/O.
- [ ] T030 [P] Write the failing tests first in `crates/depths-core/src/plan.rs`. Cover the argv table in research.md R2 for every `Operation` kind:
  - `[env-files]` present only when `global.env` exists: `--env-file ../global.env --env-file ./.env`, the second only when `.env` exists.
  - `down` with `remove_volumes`/`remove_images` adds `--volumes`/`--rmi all`, and the result is `destructive`.
  - `update` = pull + up.
  - `delete` = `down --remove-orphans` (+ directory removal only when `remove_directory`).
  - `save` with `deploy` = validate + write + up.

  Then implement it.
- [ ] T031 Define ports in `crates/depths-core/src/ports.rs`:
  - `StackStore` (list, read, write_atomic(base_hash), create, delete_dir).
  - `ContainerRuntime` (containers_by_project, run_steps streaming to a sink, validate(draft)).
  - Domain types: `Stack`, `ForeignProject` and `StackDraft`, per data-model.md.
- [ ] T032 Write the failing tests first in `crates/depths-stacks-fs/tests/`, on temp dirs. Then implement `FsStackStore` in `crates/depths-stacks-fs/src/lib.rs`. The tests:
  - Reading `../x`, an absolute name, or a file through a symlink that escapes the stack dir is refused (FR-005).
  - A symlinked stack directory is listed read-only with a reason.
  - `write_atomic` writes the exact bytes given (CRLF, BOM, trailing whitespace and comments preserved), preserves mode and owner, uses temp + rename in the same directory, and returns `Conflict{current}` when `base_hash` differs.
  - Empty `env` with no existing `.env` creates no file.
  - A compose or `.env` file larger than `DEPTHS_MAX_FILE_BYTES` (default 1 MiB) is not loaded for editing and returns `TooLarge{bytes, limit}`; the editor (T061) says why.
  - `content_hash` changes when either file changes.
  - `modified_at` is the max mtime.
  - Nothing is written when not asked (FR-003).
  - A stack dir or file Depths cannot read is listed with `read_error` and offers no edit; one it cannot write is listed with `read_only_reason`.
- [ ] T033 Define the protocol in `crates/depths-protocol/src/{cmd,event,types}.rs` exactly as in contracts/protocol.md, with ts-rs `export_to = "../../apps/depths/web/src/protocol/generated/"`. Re-export the container wire types from `container_runtime::wire`. Add `protocol_version`.
- [ ] T034 [P] Add round-trip serde tests for every command and event variant in `crates/depths-protocol/tests/roundtrip.rs`. Add `tests/export_bindings.rs`, and add a drift check (`git diff --exit-code apps/depths/web/src/protocol/generated`) to `scripts/ci.sh`.
- [ ] T035 Write `apps/depths/server/src/config.rs`: clap flags and env per contracts/config.md. Startup must refuse with a message naming the setting in each of these cases:
  - No provider is configured.
  - The public URL is not https without `--insecure-dev`.
  - `--insecure-dev` is set on a non-loopback listen.
  - The stacks dir is missing.

  - `OAUTH_CLIENT_*` is set without `OAUTH_PROVIDER`.
  - A bare `OWNER_USER_ID` value is set without `OAUTH_PROVIDER`, or an owner id names an unknown provider.

  Accept the R14 aliases (`DOCKGE_STACKS_DIR`, `ORIGIN_URL`, `OAUTH_PROVIDER` + `OAUTH_CLIENT_ID`/`OAUTH_CLIENT_SECRET`, `OWNER_USER_ID`) below the `DEPTHS_*` names: flag > `DEPTHS_*` > alias > default. `ORIGIN_URL` with no scheme means `https://`. Default listen `0.0.0.0:8080`. Log which name each setting came from, never a secret's value.

  Secrets are accepted via `*_FILE` and never logged. Add unit tests for each refusal, for alias precedence, and for a scheme-less `ORIGIN_URL`.
- [ ] T036 Write `apps/depths/server/src/users.rs`, the users registry per data-model.md and research R6:
  - Load `<data-dir>/users.json`; a missing file is an empty list, one that fails to parse refuses start with its path.
  - Write by temp file, fsync, rename, mode 0600, behind one async mutex. Depths is the only writer.
  - Merge in owners from config: always `admin`, never removable or demotable (`Denied`). Pin a `github:<login>` owner to its numeric id at first sign-in and record `pinned_from`.
  - Record every sign-in: new people as `pending` with identity and first seen; update last seen, username and email.
  - `authorise(identity, cmd, stack?)` per the data-model permission table, read fresh on each call.
  - `put`/`remove` that audit `user_added`, `user_changed`, `user_removed`, and return the sessions to end when a person, or their last stack, is removed.
  - Refuse start, naming `OWNER_USER_ID`, when no owner is set and the list holds no admin.

  Write the tests first: owner pinning (and a renamed login not matching), the start-up refusal, pending recording, an atomic write surviving a crash between write and rename (the old file is intact), a corrupt file refusing start, the lockout guard on the owner, and an assignment to a missing stack kept.
- [ ] T037 Write `apps/depths/server/src/app.rs` and `http.rs`: Axum router per contracts/protocol.md's HTTP table.
  - Use the `web-auth` session extractor and CSRF layer, with cookie `__Host-depths_session`.
  - Wire the OIDC, GitHub and Discord providers into `/auth/login`, `/auth/callback/{provider}`, `/auth/refresh` and `/auth/logout`, plus `/api/v1/me`.
  - Add `/healthz` and `/readyz`.
  - Serve static files from `--static-dir` with an SPA fallback.
  - Set security headers (CSP, nosniff, Referrer-Policy, X-Frame-Options).
  - Add integration tests: every non-exempt route without a session returns 401 (FR-028), and a mutating POST without `X-CSRF-Token` returns 403 (FR-034).
- [ ] T038 Write `apps/depths/server/src/ws.rs`: the WebSocket upgrade.
  - Check `Origin` against the public URL and require the session.
  - Handle `Hello{protocol_version, csrf_token}`, with close codes 4401, 4403 and 4429 (8 sockets per session).
  - Dispatch `CmdEnvelope`, authorising every command server-side against the users registry at the time of the command (FR-032, FR-033).
  - Implement `StreamSink` for the socket.
  - Send `SessionExpiring`.
  - Add tests for each close code, for a user's forged `Confirm` on an unassigned stack returning `Denied` and being audited, and for removing a user closing their sockets with 4403.
- [ ] T039 Write `apps/depths/server/src/ops.rs`: the operation engine.
  - Build a preview through `depths-core::plan`, and store it with a single-use id bound to `{session_id, stack_hash, expires_at = now + 5 min}`.
  - `Confirm` checks the binding, or returns `PreviewVoid{reason}`.
  - A per-stack mutex makes a second operation return `Busy{operation_id, actor}` (FR-014).
  - Run steps through `container_runtime`'s `StreamRunner`, streaming `OpOutput` and finishing with docker's stderr verbatim (FR-013).
  - `Cancel` sends SIGINT.
  - Write the audit entry before replying.
  - A stack name that fails `valid_name` or path confinement is refused with `Invalid` and writes a `denied` audit entry (spec Edge Cases).
  - Exactly one audit entry per confirmed or denied operation (SC-003).
  - Add tests with a mock runner for: name and confinement refusals audited, binding (reuse, expiry, other session, hash changed), busy, concurrent ops on different stacks, verbatim stderr, and an audit entry for denied, failed and succeeded.
- [ ] T040 Write `apps/depths/server/src/audit.rs`: the Depths `AuditEntry` per data-model.md, over `audit-log` with `RotateAtBytes`.
  - Add a test that no `.env` value or file content ever reaches an entry, with a fixture `.env` holding `DB_PASSWORD=hunter2`. Assert that `hunter2` is absent from the log file bytes (FR-021, FR-035).
- [ ] T041 Write `apps/depths/server/src/watch.rs`.
  - Start `docker events --format '{{json .}}'` and a `notify` watcher on the stacks root when the first session connects. Stop both when the last disconnects.
  - Debounce changes 250 ms into one re-inventory, and broadcast `StacksChanged`.
  - Add a test with the mock runner that no docker process is spawned while no session is connected (SC-008).
- [ ] T042 Write `apps/depths/server/src/telemetry.rs` and `main.rs`:
  - `tracing` JSON logs.
  - An OTLP exporter only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
  - Feature flags `DEPTHS_FEATURE_SHELL`/`DEPTHS_FEATURE_EDIT` exposed in `/api/v1/me`.
  - Wiring only: `main.rs` stays thin.
- [ ] T043 Write the fake backend for the web app, `apps/depths/web/tests/support/fake-backend.ts`. It is scriptable: canned stacks, previews, operation output, log and shell streams, users and pending people, and denials. It speaks the generated protocol over a mocked WebSocket, and is used by every Playwright story. Add `apps/depths/web/playwright.config.ts` with one project per story, importing the worker sizing from `ui/tests/support/workers.ts`.
- [ ] T044 Write the web shell in `apps/depths/web/src/`:
  - `protocol/transport.ts` (WebSocket, reconnect with backoff, the `Hello` handshake).
  - `app/` (router, theme via `@companion/theme`, a zustand store per feature).
  - Routes lazy-loaded with `React.lazy`.
  - A sign-in page with the configured providers, and a "you don't currently have access" page for `pending` people and users with no stacks (FR-031), showing nothing about the host.

**Checkpoint**: Depths server boots, signs in and authorises; the web shell loads.

---

## Phase 4: User Story 1, sign in and see every stack (P1) 🎯 MVP

**Goal**: a signed-in person sees every stack they may see (their assigned stacks, or all for an admin), with status, service counts and
last change, plus, for admins, foreign projects and standalone containers, read-only.

**Independent test**: three stacks (running, partial, stopped), one invalid, one foreign project.
Sign in and see each listed correctly. Then sign out and get redirected to sign-in.

- [ ] T045 [P] [US1] Write E2E tests in `apps/depths/web/tests/e2e/us1-stacks/list.spec.ts`:
  - Sign-in redirect.
  - The list shows every status in data-model.md, and a one-shot completed service does not make the stack partial.
  - Invalid stacks show the reason and have no actions; a stack with `read_error` shows the permission error and offers no edit.
  - A project started by hand with `-p` from a stack's directory is listed as "running elsewhere" under that stack, not claimed.
  - The shadowed-file warning names both files (FR-004).
  - Foreign and standalone are shown read-only.
  - Filtering by name and status.
  - At 360 px wide there is no horizontal scroll.
- [ ] T046 [P] [US1] Write a performance test in `apps/depths/web/tests/e2e/us1-stacks/scale.spec.ts`: the fake backend serves 200 stacks / 2,000 containers. Assert the first render is under 2 s and filter input is handled in under 100 ms (SC-004).
- [ ] T047 [US1] Implement `ListStacks`/`GetStack` in `apps/depths/server/src/ws.rs` via `FsStackStore` + `containers_by_project` + `depths-core::status`, filtered to the person's assigned stacks (all for an admin; `foreign` and `standalone` empty for a user). Add a test that a user never receives an unassigned stack, and that foreign projects are those with no stack dir, or a `working_dir` outside the root, and that a project whose `working_dir` is a stack's directory but whose name differs is foreign with `elsewhere_of = <stack>`.
- [ ] T087 [US1] Handle an unreachable runtime (FR-010) in `apps/depths/server/src/watch.rs` and `apps/depths/web/src/features/stacks/StackList.tsx`. When `docker ps` fails, send `RuntimeUnavailable{reason}` with docker's stderr verbatim, keep the last `Stacks` snapshot with `stale: true` and `as_of`, and show a banner. Clear it on the next successful inventory. Add a mock-runner test and an E2E case in `tests/e2e/us1-stacks/list.spec.ts`. Lifecycle previews are refused while the runtime is unavailable.
- [ ] T048 [US1] Add path-identity detection in `apps/depths/server/src/watch.rs`: warn and send a banner event when a recognised stack's `com.docker.compose.project.working_dir` is under a different root (research R1). Add a test.
- [ ] T049 [US1] Build the stack list UI in `apps/depths/web/src/features/stacks/{StackList,StackRow,StatusBadge,ForeignList}.tsx`:
  - A virtualised list using `@companion/ui` primitives.
  - Status uses both colour and text (WCAG AA).
  - Live updates from `StacksChanged`.
- [ ] T050 [US1] Build the stack detail view in `apps/depths/web/src/features/stacks/StackDetail.tsx`:
  - Services and their containers, with state, image, ports, uptime and health (FR-007).
  - The compose file read-only with highlighting (lazy CodeMirror, shared with US3).
  - `.env` with secret-looking keys masked.
- [ ] T051 [US1] Add the quickstart §3 "open the list" check, run against a real daemon with busybox fixtures, as `apps/depths/server/tests/real_daemon.rs`, behind `#[ignore]` and the env flag `DEPTHS_REAL_DAEMON=1`.

**Checkpoint**: MVP read-only stack browser.

---

## Phase 5: User Story 2, start, stop and update a stack safely (P1)

**Goal**: lifecycle actions with an exact-command preview, confirmation, live output and an audit entry.

**Independent test**: bring a stopped stack up. The preview names the stack, cwd and argv. After
confirming, the output streams and the status turns running, and the audit log has the entry.

- [ ] T052 [P] [US2] Write E2E tests in `apps/depths/web/tests/e2e/us2-lifecycle/lifecycle.spec.ts`:
  - For each of up, stop, restart, down, pull and update (stack and per service), the preview shows argv, cwd and what it reaches.
  - Cancel at the preview changes nothing.
  - Output streams live.
  - A failure shows stderr verbatim.
  - The status updates only when docker reports it (no optimistic state).
- [ ] T053 [P] [US2] Write E2E tests in `apps/depths/web/tests/e2e/us2-lifecycle/guards.spec.ts`:
  - A second operation on a busy stack shows who is running what (FR-014).
  - Destructive options are off by default and named in the preview when on (FR-015).
  - A preview made before another person's edit is refused (`PreviewVoid`).
  - The preview expires after 5 minutes.
- [ ] T054 [US2] Implement `PreviewOp`, `Confirm` and `Cancel` handling for lifecycle kinds in `apps/depths/server/src/ops.rs`, using T039's engine. Add a mock-runner test for each argv in research R2, including `global.env` flags and per-service ops carrying `[env-files]`.
- [ ] T055 [US2] Build the preview dialog in `apps/depths/web/src/features/stacks/PreviewDialog.tsx`, shared by US3:
  - Argv rendered copyable, one step per line.
  - cwd.
  - A reach list.
  - Destructive options as unchecked switches.
  - Confirm sends the `preview_id`.
  - Focus trapped, Escape cancels.
- [ ] T056 [US2] Build the operation output panel in `apps/depths/web/src/features/stacks/OperationOutput.tsx` on `@companion/terminal` `LogView`: streaming, a final outcome line, and a cancel button.
- [ ] T057 [US2] Add an ignored real-daemon test in `apps/depths/server/tests/real_daemon.rs`: `depths-test-jobs` up shows `running`, not partial, then down leaves no `depths-test-*` project (quickstart §3).

---

## Phase 6: User Story 3, edit, create and delete stacks (P2)

**Goal**: users (on their assigned stacks) and admins edit compose and `.env` with highlighting, validation, a diff, conflict detection
and byte preservation. Admins also create and delete stacks.

**Independent test**: add a service to a compose file, and check the diff shows exactly that, save
and deploy works in one preview, and every other byte is unchanged.

- [ ] T058 [P] [US3] Write E2E tests in `apps/depths/web/tests/e2e/us3-edit/edit.spec.ts`:
  - The highlight is present.
  - A broken indent gives a line-anchored error and save is disabled (FR-017).
  - A compose error from the server (`ValidationFailed`) is anchored to its line.
  - The diff shows only the change (FR-018).
  - On conflict, the other change is shown and nothing is written (FR-019).
  - Save and deploy shows the diff and `up` in one preview (FR-020).
  - `.env` secret-looking values are masked until revealed, and reveal works for a user on an assigned stack (FR-021).
- [ ] T059 [P] [US3] Write E2E tests in `apps/depths/web/tests/e2e/us3-edit/create-delete.spec.ts`:
  - Create refuses an invalid or taken name with the reason (FR-022).
  - Delete defaults to keeping the directory, and removing it is an explicit option named in the preview.
  - These actions are admin only.
  - With `DEPTHS_FEATURE_EDIT=false`, editing is hidden and the server refuses it.
- [ ] T060 [US3] Implement `PreviewSave` (validation via `ContainerRuntime::validate`, which runs `docker compose -f - config --quiet` with the draft on stdin and cwd = the stack dir), the save/create/delete kinds and `RevealEnv` (audited) in `apps/depths/server/src/ops.rs`. Add mock-runner tests that compose errors with line numbers are passed through, and that writes go through `write_atomic` with `base_hash`.
- [ ] T061 [US3] Build the editor in `apps/depths/web/src/features/editor/{ComposeEditor,EnvEditor}.tsx`:
  - CodeMirror 6 with `@codemirror/lang-yaml` and `@codemirror/lint`, fed by client-side `yaml` parse errors plus server errors.
  - Loaded with `React.lazy`.
  - A diff view via `@codemirror/merge`, or `@companion/terminal` `YamlDiffView`.
  - Unsaved-change guard on navigation.
- [ ] T062 [US3] Build the create and delete flows in `apps/depths/web/src/features/stacks/{CreateStack,DeleteStack}.tsx`, reusing `PreviewDialog`.
- [ ] T063 [US3] Add a byte-preservation check, `apps/depths/server/tests/real_daemon.rs::save_preserves_bytes`: save a compose file with comments, CRLF and odd indentation after changing one line, then confirm `sha256` of everything outside the changed line matches.

---

## Phase 7: User Story 4, work with several containers side by side (P2)

**Goal**: logs, shells and stats for several containers as tabs, split side by side, surviving reload.

**Independent test**: open logs A, a shell in B and stats for C. Arrange them, reload, and find the
layout restored and every tab live.

- [ ] T064 [P] [US4] Write E2E tests in `apps/depths/web/tests/e2e/us4-tabs/tabs.spec.ts`:
  - Open 6 tabs across kinds, split and resize.
  - Reload restores order and layout (FR-025).
  - On phone width, one pane with a tab strip (FR-024).
  - The shell sends colour, resize and Ctrl-C, and says when and why the session ended (FR-027).
  - Logs: follow, pause, search, time filter, and the "older lines dropped" marker (FR-026).
  - A stats tab samples only while visible.
  - A dropped stream shows "reconnected".
  - When the session expires mid-stream, the tab says so; after signing in again the log tabs reopen and the shell tab offers a new session.
- [ ] T065 [P] [US4] Write a soak test in `apps/depths/web/tests/e2e/us4-tabs/soak.spec.ts`: the fake backend streams 6 tabs at 50 lines/s for a simulated hour, with time compressed. Assert JS heap stays bounded (ring cap) and input latency stays under 100 ms (SC-005).
- [ ] T066 [US4] Implement `OpenLogs` (compose logs per service or `docker logs` per container, `--since` on reopen), `OpenShell`/`ShellInput`/`ShellResize` (048 `session.rs` PTY, gated by role and `DEPTHS_FEATURE_SHELL`, audited as `shell_open`), `WatchStats`/`UnwatchStats` (2 s `stats --no-stream`, only while watched) and `CloseStream` in `apps/depths/server/src/ws.rs`. Add mock-runner tests for each, including that stats stop on unwatch and on socket close.
- [ ] T067 [US4] Build the tab system in `apps/depths/web/src/features/tabs/{TabHost,TabStrip,LogsTab,ShellTab,StatsTab}.tsx`:
  - `react-resizable-panels` at wide widths, a single pane at narrow widths.
  - Layout persisted in `localStorage` under `depths:layout:<subject>`, with every read and write wrapped so the page works without storage.
  - Shells reconnect as new sessions on reload.
  - Logs resume with `since`.
- [ ] T068 [US4] Add a Vitest test in `apps/depths/web/tests/unit/layout.test.ts`: layout serialise/restore, unknown tabs dropped on restore, and a storage exception tolerated.

---

## Phase 8: User Story 5, control who can do what, and see what was done (P2)

**Goal**: a users list with an owner, user/admin roles and per-user stacks; every mutation attempt and users-list change audited; admins filter and export the log.

**Independent test**: with only an owner set, person A signs in and gets the no-access page and
shows as pending. The owner makes A a user on stack X. A's next command works on X, and A sees no
other stack, foreign project or loose container. Removing X from A ends A's session. Every change
and every attempt shows up in the audit log.

- [ ] T069 [P] [US5] Write E2E tests in `apps/depths/web/tests/e2e/us5-access/users.spec.ts`:
  - A pending person gets the no-access page (FR-031) and appears in the admin's pending list.
  - A user on X sees X only: no other stack, no foreign or standalone group, no audit or users view.
  - A user on X can reveal `.env`, run lifecycle actions, open a shell and save on X; `down` with volumes, create and delete are not offered.
  - A forged command from the user for another stack is denied and audited.
  - An admin's change to a user's stacks applies at the user's next command without signing in again; removing their last stack ends the session (FR-033).
  - The owner cannot be demoted or removed; an admin can pre-add an id that has never signed in.
  - A stack assigned but missing on disk shows as missing in the users view.
- [ ] T070 [P] [US5] Write E2E tests in `apps/depths/web/tests/e2e/us5-access/audit.spec.ts`:
  - Every mutation kind, plus denied and failed attempts, appears with actor, time, stack, argv or diff summary, and outcome (FR-035).
  - Filter by stack, actor, action and time.
  - Export downloads JSONL.
  - There are no edit or delete controls, and no API exists for them (FR-036).
- [ ] T071 [US5] Implement the GitHub provider in `crates/web-auth/src/github.rs` per research R5:
  - PKCE S256 and state.
  - Scopes `read:user user:email`; subject = the numeric id.
  - `GET /user` and `/user/emails`; no org or team calls.
  - `refresh` calls `GET /user`; a 401 is rejected.

  Add tests against a fake GitHub (axum stub) in `crates/web-auth/tests/github.rs`, including that a revoked token is rejected on refresh and that a renamed login keeps the same subject.
- [ ] T088 [US5] Implement the Discord provider in `crates/web-auth/src/discord.rs` per research R13:
  - `state` always, PKCE S256 sent; nothing relies on PKCE alone.
  - Scopes `identify email`; redirect URI exactly `<public_url>/auth/callback/discord`.
  - `GET /users/@me` → subject (the snowflake), username, verified email. No guild calls.
  - Refresh with the refresh token; 401 = rejected, 429 = unreachable (honour `retry_after`).

  Add tests against a fake Discord (axum stub) in `crates/web-auth/tests/discord.rs`: no guild endpoint is ever called, a 401 on refresh is rejected, and a 429 keeps the session. Then confirm the PKCE exchange once against a real Discord application and record the result in research R13.
- [ ] T072 [US5] Implement session refresh scheduling in `apps/depths/server/src/http.rs`: every 15 min, and on `/auth/refresh`. Revalidate the token through the session's provider. When the provider **rejects** it (revoked grant, 401/403), end the session (fail closed). When the provider is **unreachable** (connect error, timeout, 5xx), keep the session until it expires, retry with backoff, and log a warning (spec Edge Cases). Add a test for each case.
- [ ] T073 [US5] Implement `GetAudit`, `GetUsers`, `PutUser`, `RemoveUser` and `GET /api/v1/audit/export` (admin only) in `apps/depths/server/src/{ws,http}.rs`, with cursor paging over `audit-log`'s reader. Broadcast `UsersChanged` to admin sockets; on a removal, end the returned sessions and close their sockets.
- [ ] T074 [US5] Build the admin views in `apps/depths/web/src/features/{audit/{AuditLog,AuditFilters},users/UsersView}.tsx`: the audit log as a virtualised table that collapses to cards at narrow widths, with export. UsersView lists users and the pending list, sets role and stacks (missing stacks marked), adds an id by hand, and shows owners locked.
- [ ] T075 [US5] Write a security sweep in `apps/depths/server/tests/sweep.rs`. It walks every route in the router and every protocol command, as unauthenticated, pending, a user on an assigned stack, a user on an unassigned stack, and admin, and asserts that the outcome matches the permission table in data-model.md (SC-006).

---

## Phase 9: User Story 6, deploy Depths as one container, least privilege by default (P3)

**Goal**: one image, an example compose with a socket proxy, non-root, no auto-restart.

**Independent test**: on a clean host, follow the guide with the example compose. Sign in and
manage a stack through the proxy. Check that Depths runs as non-root with no socket mounted.

- [X] T076 [US6] Write `apps/depths/Dockerfile`, multi-stage per research R11:
  - A cargo-chef planner/cook/build stage on `rust:1.99.0-trixie`, with the version read from `rust-toolchain.toml` via a build arg.
  - A `web` stage on Node 24 + corepack, running `pnpm --filter @companion/depths-web build`.
  - A `cli` stage copying `docker` and the compose plugin from `docker:29-cli`.
  - A runtime stage on `debian:trixie-slim` with `tini` and user `depths` (10001), assets at `/usr/share/depths/web` passed as `STATIC_DIR` (not embedded), and `HEALTHCHECK` on `/healthz`.
  - Add `.dockerignore` entries.
- [ ] T077 [US6] Write `apps/depths/compose.yaml`, and `apps/depths/.env.example` (with `OWNER_USER_ID`) per contracts/config.md:
  - A linuxserver socket-proxy with the R8 env.
  - Socket mounted read-only on the proxy only.
  - The stacks dir at the same path on both sides.
  - `user: "10001:10001"`.
  - `restart: ${RESTART_POLICY:-no}` on both services.
  - A header comment stating that socket access is root-equivalent.
- [ ] T078 [US6] Add a container test in `apps/depths/tests/container.sh`, called by `make depths-test-image`:
  - Build once, then `up`, and check that `id -u` in the Depths container is 10001 and that `/var/run/docker.sock` is absent from it.
  - Check that `/readyz` is 200 through the proxy, and that a missing provider config refuses to start.
  - Open an exec through the proxy and keep it idle past the proxy's 10-minute cut. The cut is configurable in the test via the proxy's timeout env, so it is short here. Check that the tab reconnects.
  - Then `down`, prune build cache (`--filter until=24h`), and run `scripts/hygiene.sh`.
- [ ] T089 [US6] Deploy beside Dockge on the steam host (first real deployment):
  - `../ansible/composes/steam/compose.yaml`: `depths-socket-proxy`, `depths` on 8080 (stacks dir at the same path, `user: "1001:1001"` so files stay steam's, explicit env mapped from the host `.env` including `OWNER_USER_ID=140642000969007104` with `OAUTH_PROVIDER=discord`, no `env_file` passthrough of unrelated secrets), and a `depths-tailscale` sidecar (`TS_AUTHKEY`, Headscale login server, `TS_SERVE_CONFIG` forwarding tailnet TCP 8080 to `depths:8080`).
  - `../fluxified/overlays/system/tailscale/`: `egress-depths.yaml`, `service-depths.yaml` (port 8080 only), `route-depths.yaml` for `depth.boop.ninja`, the `ts-state-egress-depths` RBAC name, a `secrets/tailscale-egress-depths.sops.env` generator, and the listener pair on `shared-gateway`.
  - Register `https://depth.boop.ninja/auth/callback/discord` in the Discord application.
  - Pushing fluxified deploys prod, and the compose change is applied on the host: both wait for the owner's go-ahead.
- [ ] T079 [US6] Write the Dockge round-trip check as a script, `apps/depths/tests/dockge-roundtrip.sh`. Take checksums of a copied stacks dir, run Depths lifecycle actions only, take checksums again, and confirm they are identical (SC-002). Running Dockge itself is manual, documented in quickstart §4.

---

## Phase 10: User Story 7, find both apps as one suite (P3)

**Goal**: the README presents Fathom and Depths as one orchestration companion suite.

**Independent test**: read the README cold. It names both apps, says which to pick for Kubernetes
and which for a Docker host, and links each app's install.

- [ ] T080 [US7] Rebrand `README.md` as the orchestration companion suite. Keep it light:
  - One paragraph on the suite.
  - A two-row table (Fathom: Kubernetes, desktop and hosted; Depths: Docker/Compose hosts, one container).
  - Install links.
  - Links to `docs/guides/` and `CONTRIBUTING.md`.
  - Keep Fathom's existing screenshots, and add a Depths placeholder until T086.
- [ ] T081 [P] [US7] Write user guides in `docs/guides/depths/`:
  - `README.md`.
  - `install.md` (example compose, same-path mount, socket proxy, the root-equivalence warning, `RESTART_POLICY`).
  - `sign-in.md` (OIDC; GitHub OAuth App, with org restriction approval).
  - `users.md` (`OWNER_USER_ID`, finding your Discord or GitHub id, pending people, assigning stacks).
  - `migrating-from-dockge.md` (point Depths at the same dir, file precedence difference, `global.env`, what Depths writes and does not, switching back).
  - `tabs-and-shells.md`.

  Link them from `docs/guides/README.md`.
- [ ] T082 [P] [US7] Extend `CONTRIBUTING.md`:
  - Workspace layout (`apps/`, `crates/`, `packages/`, Fathom's existing roots and why they stay).
  - The shared-crate rules (no `fathom`/`kube` deps, the bindings drift checks).
  - The Depths dev loop (`make depths-dev`, fake OIDC under `--insecure-dev`).
  - The test layers and the resource rules.

---

## Phase 11: Polish & cross-cutting

- [ ] T083 [P] Add `THIRD_PARTY_LICENSES.md` / `scripts/third-party-licenses.sh` coverage for the new crates and packages (CodeMirror, `notify`, OTel, `react-resizable-panels`), and add `license` fields in every new `Cargo.toml` and `package.json`.
- [ ] T084 [P] Add an accessibility pass over Depths: axe checks in each Playwright story project (`apps/depths/web/tests/support/axe.ts`), keyboard-only flows for preview and confirm, and contrast via `@companion/theme` color tests.
- [ ] T085 Run `quickstart.md` §1–§5 end to end and record the results, including the wall-clock time from clean host to first sign-in (SC-001, target under 10 minutes), in `specs/050-depths-stack-manager/quickstart.md` under a "Validated" note. Then run `scripts/hygiene.sh` and `docker builder prune --filter until=24h`.
- [ ] T086 (last) Make the brochure and README screenshots of Depths: the stack list, preview, editor diff, tabs split, phone layout and audit. Take them from the dev server with fixture stacks only. Screen each one for hostnames, emails, tokens and real stack names before committing to `docs/guides/depths/img/` and the README.

---

## Dependencies & execution order

```
T001 ─┬─ T002 ─ T003 ─────────────┐
      ├─ T004 ────────────────────┤
      └─ T005, T006 (P) ──────────┤
                                  ├─ B1 (T007–T013) ─┐
                                  ├─ B2 (T014–T020) ─┼─ Phase 3 (T026–T044) ─ US1 ─ US2 ─┬─ US3
                                  └─ B3 (T021–T025) ─┘                                   ├─ US4
                                                                                         └─ US5
                                                         US6 after US2 · US7 any time after T001 · T086 last
```

- B1, B2 and B3 are independent of each other, but each needs T002–T004.
- In Phase 3, T026–T030 are parallel (separate files in `depths-core`). T031 needs them. T032 needs T031. T033 needs B1. T035–T042 need B2 and T031–T034. T043 and T044 need B3 and T033.
- US1 and US2 are P1 and come first. US3, US4 and US5 depend only on US2's preview dialog and engine (T054, T055) and can run in parallel after it.
- T071 (GitHub provider) and T088 (Discord provider) can start any time after B2.
- T076, T077 and T089 are pulled forward to run right after US1, with T088, so the read-only MVP can be tried on the steam host. The rest of US6 keeps its place.

## Parallel examples

- **Extraction**: three agents in worktrees: B1 `container-runtime`, B2 `web-auth` + `audit-log`, B3 `@companion/*`. Merge each when its gate (T013, T020, T025) is green.
- **Domain core**: T026, T027, T028, T029 and T030 at once (one file each).
- **After US2**: US3, US4 and US5 as three agents. Their server work touches different handlers in `ws.rs`/`ops.rs`, so land them in turn and rebase small.
- **Docs**: T081 and T082 alongside any story phase.

## Implementation strategy

1. **MVP = Setup + Foundational + Phase 3 + US1**: a signed-in, read-only stack browser. This proves the extraction, auth and the protocol end to end with nothing that can change the host.
   Then T088, T076, T077 and T089: Discord sign-in, the image and the steam deployment, so the MVP
   runs beside Dockge at `https://depth.boop.ninja`.
2. **Add US2**: Depths is now a usable Dockge replacement for day-to-day lifecycle.
3. **Add US3, US4 and US5** in parallel: editing, tabs, users and audit.
4. **Add US6 and US7**: ship the image and the suite README.
5. **Finish with T085 and T086**: validation, then screenshots last.

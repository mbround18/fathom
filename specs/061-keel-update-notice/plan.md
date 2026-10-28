# Implementation Plan: Keel update notice and self-update

**Branch**: `main` (agent work lands on worktree branches, merged into `main`) | **Date**: 2026-10-28 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/061-keel-update-notice/spec.md`

## Summary

Keel learns what it is running, can tell owners when a newer image is published, and can update
itself through one reviewed run.

1. **Build identity (US1)**:
   - The Dockerfile passes version, revision, build date and source to the binary as
     `KEEL_BUILD_*`, from the same args as the image labels.
   - The role passes the deployed digest as `KEEL_IMAGE_DIGEST` (R2, R3).
   - The System tab shows them to owners and admins. Users see the version under About.
2. **Update check, off by default (US2)**:
   - A small OCI registry client, behind `KEEL_UPDATE_CHECK=true`.
   - It HEADs the watched tag and reads labels only when the digest is new. That is zero pulls in
     the steady state, and one per newly published digest (R1, R8).
3. **Notice (US3)**:
   - An in-memory status on a `watch` channel, pushed live to owners (full) and admins (one line).
   - Owners get a nav dot, dismissed per digest in one new `users` column (R5, R6, R7, R10).
4. **Self-update (US4)**:
   - A preview with passkey step-up queues the System workspace's `keel-server` run, pinned to the
     previewed digest.
   - This uses a new, narrow mechanism: **job params** that the signed repo declares and Keel fills
     (R4).
   - Audited request, result and digest change.
5. **Manual path (US5)**: when a prerequisite is missing, the card names it and shows the digest
   reference and commands.

## Technical Context

**Language/Version**: Rust stable (edition 2024) for keel-core, keel-protocol, keel-exec,
keel-server and keel-runner. TypeScript, React 19 and Vite for `apps/keel/web`.

**Primary Dependencies**:
- Existing: `axum`, `tokio`, `sqlx`, `clap`, `opentelemetry`, `tracing-opentelemetry`, `ts-rs`,
  `serde_json`, `time`, `url`.
- **Proposed, needs owner approval**: `reqwest = { workspace = true }` as a *direct* dependency of
  `keel-server`. It is already in the workspace at 0.13.5, and already compiled into keel-server
  through `web-auth`, so nothing new enters the lockfile or the build. Alternatives are in R1:
  `oci-client` 0.18 (also in the lockfile, but its convenience calls count pulls), or `hyper-util`
  by hand.
- No new web dependencies. No new test dependencies: the fake registry is an axum router.

**Storage**: PostgreSQL, through migration `0011_updates.sql` (data-model.md):
- `users.update_dot_dismissed`;
- `runs.params`;
- the single-row `keel_build_seen`.

Check results stay in memory.

**Testing**:
- `cargo test` for each crate touched;
- the keel-server registry tests against an in-process fake registry;
- the keel-server DB tests (throwaway PostgreSQL 18);
- runner tests for params;
- Vitest;
- a new Playwright project, `s061`, with one spec per story, against the fake backend.

Everything is deterministic: paused tokio time, seeded jitter, the existing `Clock`, and no real
network.

**Target Platform**: The Keel server as a Linux container (amd64 today, arm64 when the image is
built for it). The runner on Debian hosts. Evergreen browsers.

**Project Type**: An existing web service, agent and web app (spec 055).

**Performance Goals**:
- A check makes at most 3 requests when nothing changed, and at most 6 when something did.
- 30 s worst case.
- No per-socket polling: one `watch` value fans out to sockets.

**Constraints**:
- No outbound request with checks off.
- Never a counted pull for an unchanged tag.
- Credentials are env- or file-only, and never logged.
- Every update is preview, step-up, run and audit. No automatic mode (FR-022).
- Params are declared by the signed repository, never free-form.

**Scale/Scope**:
- One migration.
- 5 new commands, 3 new events, 2 changed types, and one changed frame field.
- A manifest field.
- A Dockerfile and Makefile change.
- One System tab panel and card, plus a nav dot.
- One contract for the owner's Ansible role.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Assessment |
|---|---|
| I. Operator-first, explicit mutation | Pass. The only state-changing action, Update, is a preview with step-up, re-validated on confirm, and runs as an ordinary System run under its approval policy. Checks are off until the operator turns them on in deployment config. Dismissing the dot is a display preference with no preview, like other per-user display state. There is no automatic update. |
| II. One contract per app | Pass. New commands, events, `Me`, `RunInfo` and the `Assign.params` field all live in `keel-protocol` or `keel-exec`, with regenerated bindings. The manifest's `params` lives in `keel-core`, shared by server and runner. The web never builds URLs or digest references; the server sends them. |
| III. Tested at the boundaries | Pass. Round-trip tests for each new variant. A bindings drift check. Registry behaviour is pinned by contracts/registry.md and asserted against the fake registry's request log. Old and new `Assign` serialization. Runner refusal on params mismatch. Playwright `s061`, one spec per story, against the fake backend. |
| IV. Density scales with width | Pass. The build panel is a definition list, and the card stacks at 360 px. The impeccable pass covers both at 360 and 1280 px. |
| V. Fast and quiet | Pass. The default is no network. With checks on, one HEAD every 6 h. No web polling: status is pushed. No new web dependency or chunk beyond the existing lazy settings page. |
| VI. User data is sacred | Pass. Audit stays insert-only, and every request, refusal, result and digest change is recorded with the old and new digest. Registry credentials are never logged or put in events. `keel_build_seen` has no delete grant. Rollback is kept possible through `digest.previous`. |

**Post-design re-check**: passes, with two items recorded under Complexity Tracking:

- the job-params mechanism, a new cross-cutting feature this spec needs;
- the direct `reqwest` dependency, which is pending approval.

Neither replaces a stack component. Both stay within Principle II.

## Project Structure

### Documentation (this feature)

```text
specs/061-keel-update-notice/
├── plan.md
├── research.md            # R1–R10
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── protocol.md        # Cmd/Event, Me, RunInfo, Assign.params, manifest params
│   ├── config.md          # KEEL_UPDATE_*, KEEL_IMAGE_DIGEST, KEEL_BUILD_*
│   ├── registry.md        # exact request set, fake registry
│   └── ansible-role.md    # keel_server role change (owner's repo, not edited here)
└── tasks.md               # /speckit-tasks
```

### Source code

```text
crates/keel-protocol/src/{cmd,event,types,run}.rs     # new Cmds/Events, Me, RunInfo.params
crates/keel-protocol/tests/roundtrip_061.rs           # new
crates/keel-core/src/manifest.rs                      # params: ParamSpec, kind oci_digest
crates/keel-exec/src/frames.rs                        # Assign.params (skip when empty)
apps/keel/server/migrations/0011_updates.sql          # new
apps/keel/server/src/build_info.rs                    # BuildInfo::from_env (new)
apps/keel/server/src/updates/mod.rs                   # Updates state, scheduler, classify (new)
apps/keel/server/src/updates/registry.rs              # OCI client (new)
apps/keel/server/src/updates/commands.rs              # Get*, CheckForUpdates, Dismiss, PreviewKeelUpdate (new)
apps/keel/server/src/{config,serve,ws,dispatch,telemetry}.rs
apps/keel/server/src/{runs,system,dispatch}.rs        # NewRun.params, update run, Me fields, keel_updated at start
apps/keel/server/tests/support/fake_registry.rs       # new
apps/keel/server/tests/updates.rs                     # new
apps/keel/runner/src/executor/mod.rs                  # params.json, -e order, mismatch refusal
apps/keel/Dockerfile                                  # ARG CREATED, empty VERSION default, ENV KEEL_BUILD_*, created label
apps/keel/Makefile                                    # VERSION from keel-v* tag, CREATED
apps/keel/compose.yaml                                # KEEL_UPDATE_*, KEEL_IMAGE_DIGEST passthrough
apps/keel/web/src/features/settings/{tabs.ts,SystemTab.tsx,store.ts}
apps/keel/web/src/features/settings/{BuildPanel,UpdateCard,UpdateLine}.tsx   # new
apps/keel/web/src/app/{nav.ts,Shell.tsx}              # dot on System Settings, About version
apps/keel/web/playwright.config.ts                    # s061 project
apps/keel/web/tests/{unit,e2e/s061}/
docs/guides/keel/updates.md                           # turning checks on, updating, rollback
```

**Structure decision**:
- **The main session lands the contract first**:
  - protocol types and bindings;
  - `Assign.params`;
  - manifest `params`;
  - the migration;
  - `BuildInfo`;
  - the config settings.
- **Then parallel agents take disjoint areas**:
  1. registry client and scheduler (`updates/registry.rs`, `updates/mod.rs`, fake registry);
  2. commands, socket push, the update run and audit (server);
  3. runner params;
  4. web (panel, card, dot, `s061` specs);
  5. Dockerfile, Makefile, compose and the guide.
- **Last**: the impeccable pass over the System tab and nav. The owner's Ansible role change
  follows the rollout order in quickstart.md.

## Complexity Tracking

| Item | Why needed | Simpler alternative rejected because |
|---|---|---|
| Job params (manifest `params`, `runs.params`, `Assign.params`, runner `-e @params.json`) | FR-017 requires the run to deploy the exact previewed digest. Today every variable comes from the repo at the pinned commit, so the server has no way to pass one. | The role resolving the tag at run time could deploy a different image from the one previewed. A secret would hide the value from audit. A managed inventory is refused in signed mode. Keel committing to the repo needs write and signing power. A free-form `extra_vars` would let a compromised server override anything (R4). |
| `reqwest` as a direct keel-server dependency | The registry check needs an HTTPS client with TLS and redirects. | It is already compiled into keel-server through `web-auth`, so there is no build cost. `oci-client` makes unchanged-tag checks count as pulls. Raw hyper means writing TLS and redirect handling. **Pending owner approval.** |

## Spec issues

These were found while planning. The plan's handling is noted for each.

1. **Admins and digests (FR-002, US1-1 vs SC-006, FR-015)**:
   - FR-002 shows admins the running digest; SC-006 says admins never see "a digest".
   - The plan shows admins the running digest and never the available one.
   - Suggested wording for SC-006: "never see an available digest or an Update action".
2. **System tab visibility**: spec 060 made the System tab owner-only, but US1 has admins opening
   it. The plan opens the tab to admins, with role-shaped sections (R10).
3. **SC-005, "zero pulls"**: this is only reachable while nothing new is published. Reading labels
   (FR-006) needs a manifest GET, which Docker Hub counts. The plan's floor is one pull per newly
   published digest, per process. Suggested wording: "no pulls while the tag is unchanged, and at
   most one per newly published image".
4. **Release versions don't exist yet**: `VERSION` is always `0.1.0` (Cargo.toml) today, and there
   are no `keel-v*` tags. Until one is cut, every image is "untagged", and no card has release
   notes (R2, R7).
5. **Single architecture**: the image is built with plain `docker build`, for one platform. The
   constitution names amd64 and arm64. The check handles indexes and single manifests both, but an
   arm64 Keel would see `cant_tell` until the image is built multi-arch.
6. **FR-017 needs a new mechanism**: job params cross keel-core, keel-exec, the runner, the server
   and the owner's repository. It is larger than "queue a run" suggests, and it has a rollout order
   (quickstart.md).
7. **FR-003 is in another repository**: the plan writes it as a contract (contracts/ansible-role.md).
   The owner lands it in the Ansible repository. The role's current `pull: missing` with
   `:latest` never picks up a newer image even by hand. The contract fixes that too.

## Open risks

- **Downgrade by a compromised server**:
  - Params let Keel choose any digest of the configured repository, including an old, vulnerable
    one.
  - Mitigations: owner step-up on every request; old and new digests in the audit; the System
    workspace's approval policy; the preview warns on `older_build`.
  - Image signature verification (cosign) would close it, and is out of scope.
- **Rollout order**: if the repository adds `params:` before Keel and the runner support it,
  `deny_unknown_fields` breaks the `keel-server` job until a manual update. The guide and
  quickstart.md state the order.
- **Self-restart mid-run**: the run updates the host Keel itself runs on.
  - It relies on spec 058's deferred restart and spec 060's late-result replay.
  - If the new image fails to start, nothing in Keel can report it.
  - The owner falls back to `make keel-server KEEL_DIGEST=…` with `digest.previous`.
- **Docker Hub behaviour**: that HEAD is free and GET is counted is Docker Hub's documented
  policy, which can change. The `keel.update.pulls` counter lets the operator see it.
- **Label trust**: version, date and source labels come from whatever was pushed to the tag.
  - Classification only informs the card; it never acts alone.
  - The release-notes link requires the source to match the running build (R7).

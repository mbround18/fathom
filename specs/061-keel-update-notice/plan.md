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
   - A new shared crate, `crates/oci-registry`: a HEAD-first OCI distribution client on the
     `reqwest` already in the workspace, with no Keel or Depths types. Spec 062 uses it too (R1,
     contracts/registry.md).
   - Keel calls it only behind `KEEL_UPDATE_CHECK=true`. It HEADs the watched tag and resolves
     labels only when the digest is new. That is zero pulls while the tag is unchanged, and at most
     one per newly published image, per process (R1, R8, SC-005).
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
- **New crate, `crates/oci-registry`** (approved, shared with spec 062). keel-server depends on it
  by path. Its dependencies are all already in the lockfile, at the versions sibling crates use:
  - `reqwest` 0.13.5 with `json` and `default-tls`, the same as `web-auth` (and the root `fathom`
    package). reqwest comes in through the workspace: it is already compiled into keel-server
    through `web-auth`, so this adds no new dependency edge;
  - `secrecy` 0.10, `thiserror` 2, `serde` 1.0.229, `serde_json` 1.0.154, `sha2` 0.11 (body
    digest checks), `tracing` 0.1.44, `tokio` 1.54.1;
  - behind its `test-util` feature only: `axum` 0.8, for the in-process fake registry.
- The workspace `Cargo.toml` has no `[workspace.dependencies]` table, so `{ workspace = true }`
  isn't available. Each crate declares the version it uses, as every sibling crate does today.
- No new web dependencies. No new test dependencies: the fake registry is an axum router.

**Storage**: PostgreSQL, through migration `0011_updates.sql` (data-model.md):
- `users.update_dot_dismissed`;
- `runs.params`;
- the single-row `keel_build_seen`.

Check results stay in memory.

**Testing**:
- `cargo test` for each crate touched;
- the `oci-registry` tests against its in-process fake registry (`test-util` feature), asserting
  the exact request log; keel-server's check tests reuse the same fake;
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
- One new library crate, `crates/oci-registry`, shared with spec 062.
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
| III. Tested at the boundaries | Pass. Round-trip tests for each new variant. A bindings drift check. Registry behaviour is pinned by contracts/registry.md and asserted in `oci-registry`'s own tests against the fake registry's request log. Old and new `Assign` serialization. Runner refusal on params mismatch. Playwright `s061`, one spec per story, against the fake backend. |
| IV. Density scales with width | Pass. The build panel is a definition list, and the card stacks at 360 px. The impeccable pass covers both at 360 and 1280 px. |
| V. Fast and quiet | Pass. The default is no network. With checks on, one HEAD every 6 h. No web polling: status is pushed. No new web dependency or chunk beyond the existing lazy settings page. |
| VI. User data is sacred | Pass. Audit stays insert-only, and every request, refusal, result and digest change is recorded with the old and new digest. Registry credentials are never logged or put in events. `keel_build_seen` has no delete grant. Rollback is kept possible through `digest.previous`. |

**Post-design re-check**: passes, with two items recorded under Complexity Tracking:

- the job-params mechanism, a new cross-cutting feature this spec needs;
- the new `crates/oci-registry` crate, an adapter shared with spec 062.

Neither replaces a stack component. Both stay within Principle II. The crate follows the agnostic
core plus adapters rule: it holds no Keel or Depths types, and each app maps its results.

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
│   ├── registry.md        # crates/oci-registry API, exact request set, fake registry
│   └── ansible-role.md    # keel_server role change (owner's repo, not edited here)
└── tasks.md               # /speckit-tasks
```

### Source code

```text
crates/oci-registry/Cargo.toml                        # new crate, shared with spec 062
crates/oci-registry/src/{lib,reference,auth,client,manifest,error}.rs   # ImageRef, Credentials, Client, Resolved
crates/oci-registry/src/testing.rs                    # FakeRegistry (feature test-util)
crates/oci-registry/tests/client.rs                   # request-log tests against the fake
crates/keel-protocol/src/{cmd,event,types,run}.rs     # new Cmds/Events, Me, RunInfo.params
crates/keel-protocol/tests/roundtrip_061.rs           # new
crates/keel-core/src/manifest.rs                      # params: ParamSpec, kind oci_digest
crates/keel-exec/src/frames.rs                        # Assign.params (skip when empty)
crates/keel-trust/src/challenge.rs                    # OverrideScope.params (omitted when empty)
apps/keel/server/migrations/0011_updates.sql          # new
apps/keel/server/src/build_info.rs                    # BuildInfo::from_env (new)
apps/keel/server/src/updates/mod.rs                   # Updates state, scheduler, classify (new)
apps/keel/server/src/updates/check.rs                 # one check on oci-registry: cache, classify input, counts (new)
apps/keel/server/src/updates/commands.rs              # Get*, CheckForUpdates, Dismiss, PreviewKeelUpdate (new)
apps/keel/server/src/{config,serve,ws,dispatch,telemetry}.rs
apps/keel/server/src/{runs,queue,system,dispatch}.rs  # NewRun.params, Assign.params, update run, Me fields, keel_updated at start
apps/keel/server/tests/updates.rs                     # new
apps/keel/runner/src/executor/mod.rs                  # params.json, -e order, mismatch refusal
apps/keel/Dockerfile                                  # ARG CREATED, empty VERSION default, ENV KEEL_BUILD_*, created label
apps/keel/Makefile                                    # VERSION from keel-v* tag, CREATED
apps/keel/compose.yaml                                # KEEL_UPDATE_*, KEEL_IMAGE_DIGEST passthrough
apps/keel/server/Cargo.toml                           # oci-registry (and test-util as a dev-dependency)
apps/keel/web/src/features/settings/{tabs.ts,SystemTab.tsx,store.ts}
apps/keel/web/src/features/settings/{BuildPanel,UpdateCard,UpdateLine}.tsx   # new
apps/keel/web/src/app/{nav.ts,Shell.tsx}              # dot on System Settings, About version
apps/keel/web/playwright.config.ts                    # s061 project
apps/keel/web/tests/{unit,e2e/s061}/
docs/guides/keel/updates.md                           # turning checks on, updating, rollback
```

**Structure decision**:
- **The shared crate first**: `crates/oci-registry`, with its fake registry and tests, unless spec
  062's implementation has already built it. If it exists, 061 uses it as it is, and adds to it
  only through the contract in contracts/registry.md.
- **The main session lands the contract first**:
  - protocol types and bindings;
  - `Assign.params`;
  - manifest `params`;
  - the migration;
  - `BuildInfo`;
  - the config settings.
- **Then parallel agents take disjoint areas**:
  1. the update check and scheduler on `oci-registry` (`updates/check.rs`, `updates/mod.rs`);
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
| A new crate, `crates/oci-registry` | Keel (061) and Depths (062) both need a registry check that never counts a pull for an unchanged tag, with an auditable request set. | Two hand-rolled clients would drift. `oci-client` makes unchanged-tag checks count as pulls. Raw hyper means writing TLS and redirect handling. The crate is written on the `reqwest` already compiled into both servers, so it adds no dependency edge. Approved by the owner. |

## Resolved with owner (2026-10-28)

The plan's spec issues were settled with the owner and applied to the spec:

1. **Admins and digests**: SC-006 now says admins may see the running build, digest included, but
   never the available update's digest, details or actions. FR-015 and US3-5 match.
2. **System tab visibility**: the tab opens to admins with role-shaped sections (R10, FR-024). This
   amends spec 060; its owner-only sections stay owner-only.
3. **SC-005**: zero pulls while the tag is unchanged, at most one counted pull per newly published
   image, per process. FR-006 matches.
4. **Run params**: accepted as designed (R4), now FR-023, with the rollout order in the spec's
   Assumptions and in quickstart.md.
5. **Registry client**: the shared `crates/oci-registry` crate (R1), replacing the hand-rolled
   client and the separate `reqwest` approval.
6. **Known conditions, recorded in the spec's Assumptions and Edge cases**: no `keel-v*` tags yet,
   a single-architecture image, FR-003 delivered in the Ansible repository, and the role's
   `pull: missing` fixed by FR-003.

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
  policy, which can change. `oci-registry` reports request counts by method on every call, and
  Keel's `keel.update.pulls` counter lets the operator see it.
- **Shared crate, two consumers**: a change to `oci-registry` for 062 can change Keel's request
  set. contracts/registry.md is the contract both specs hold it to, and its request-log tests live
  in the crate.
- **Label trust**: version, date and source labels come from whatever was pushed to the tag.
  - Classification only informs the card; it never acts alone.
  - The release-notes link requires the source to match the running build (R7).

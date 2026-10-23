# Implementation Plan: Depths, a self-hosted Compose stack manager

**Branch**: `050-depths-stack-manager` | **Date**: 2026-10-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/050-depths-stack-manager/spec.md`

## Summary

Depths is a web app, shipped as one container, that manages the Compose stacks in a
Dockge-layout stacks directory. It is the second app in the suite, next to Fathom. The stacks
directory stays the only record of what a stack is, so an operator can switch between Dockge and
Depths in either direction.

Two decisions shape the implementation:

1. **Depths shares Fathom's code rather than copying it.** Three pieces move out of Fathom into
   workspace crates under `crates/`:
   - Fathom's Docker/Compose runtime from spec 048, which drives the docker CLI and previews the
     exact argv.
   - The hosted server's sign-in, session and CSRF code.
   - The hosted server's append-only audit log.

   Fathom re-exports what moved, so its protocol, generated TypeScript and tests do not change.
   The frontend's theme, shadcn primitives and terminal/log surfaces likewise move into pnpm
   packages under `packages/`. Depths is then a thin server and a web app in `apps/depths/` built
   on those.
2. **Every mutation goes through Fathom's guard.** This is the 048 preview with the exact command,
   followed by confirmation, a server-side role check, and an audit entry naming the signed-in
   person. No path skips the preview. The difference from 048 is the gate. In 048 a project is
   read-only until it is adopted. In Depths, placing a directory in the stacks dir already makes it
   manageable, and per-stack roles from the IdP's groups decide who may act on it.

See [research.md](./research.md) for the Dockge behaviour, the crate seams and the rejected
alternatives.

## Technical Context

**Language/Version**:
- Rust 1.99.0 (pinned in `rust-toolchain.toml`), edition 2024, in the existing Cargo workspace.
- TypeScript 6 / React 19.3, Vite 8, Tailwind 4, in a root pnpm 12 workspace.

**Primary Dependencies**:
- Existing: `tokio`, `axum` + `tower`/`tower-http`, `serde`, `ts-rs` 12, `clap`, `tracing`,
  `openidconnect` (as used by `server/`), `@xterm/xterm` 6, `@tanstack/react-virtual`, `yaml`,
  `diff`, shadcn/radix, zustand.
- New, Rust:
  - `notify`, for watching the stacks dir (R9).
  - `tracing-opentelemetry` and `opentelemetry-otlp`, an optional exporter (R12).
  - `toml`, for the roles file, if it is not already in the lock.
- New, web:
  - `@uiw/react-codemirror`, or CodeMirror 6 directly: `@codemirror/lang-yaml`, `@codemirror/lint`
    and `@codemirror/merge`. Loaded lazily (R10).
  - `react-resizable-panels`, for side-by-side tabs.

**Storage**:
- No database.
- Stacks are the files in the stacks dir.
- Depths' own state lives in `DEPTHS_DATA_DIR` (default `/var/lib/depths`):
  - The audit log, `audit/audit-log.jsonl` plus rotated files.
  - Nothing else.
- Sessions are in memory.
- The roles file is read-only to Depths.

**Testing**:
- `cargo test`: unit tests against a mock `Runner` with canned docker output, as in 048.
- `cargo test`: a round-trip test for each `depths-protocol` command and event.
- `cargo test`: filesystem adapter tests on temp dirs (confinement, symlinks, conflicts, byte
  preservation).
- `cargo test`: auth tests against an in-process fake OIDC issuer and a fake GitHub (wiremock-style
  axum stubs).
- Vitest for pure TS: status mapping, tab layout persistence, and the env masking.
- Playwright E2E per user story against a scripted fake Depths backend, segmented one project per
  story.
- A real-daemon layer in the quickstart: busybox fixtures, one exec-through-socket-proxy check.

**Target Platform**:
- A Linux container (amd64, arm64) next to a Docker Engine 25+ with Compose v2.
- Browsers: current Chromium, Firefox and Safari, at phone through desktop widths.

**Project Type**: a web service (Axum) plus a single-page web app (React), and shared libraries.

**Performance Goals**:
- SC-004: 200 stacks / 2,000 containers listed in under 2 s, and interactions handled in under
  100 ms.
- SC-005: 6 open tabs streaming for an hour with bounded memory.

**Constraints**:
- SC-008: idle under 2% CPU with no one signed in, which means no docker invocation at all.
- Never rewrite an operator's file beyond what they typed (Principle VI).
- No anonymous access (FR-028).
- Runs as a non-root user (FR-042).
- Assets are served from `--static-dir`, never embedded.

**Scale/Scope**:
- 7 user stories, 46 functional requirements, 8 success criteria.
- 3 extracted crates and 3 Depths crates.
- 3 shared web packages and 1 Depths web app.

### Resource budget for development and testing

The machine is shared with the operator's work and with other agents:

- **Fixtures use one image, `busybox`.**
  - Stacks for tests are created under a temp stacks dir, with project names prefixed
    `depths-test-`.
  - They are removed by name with `docker compose down`, never by prune.
- **Almost nothing needs a daemon.** Discovery, status, planning, authorisation and the protocol
  are unit-tested against a mock `Runner` and temp dirs. The UI is tested against the fake
  backend. Only the quickstart's real-daemon checks touch Docker.
- **No kind cluster** is created for this feature.
- **`CARGO_BUILD_JOBS=4`.** Prefer `cargo check -p <crate>` while iterating. Playwright sizes its
  own workers (`ui/tests/support/workers.ts`), and the Depths config reuses that helper.
- **Image builds**:
  - The Depths image is built at most once per task group.
  - After a batch, run `docker builder prune --filter until=24h`.
  - Never remove tagged images, containers or volumes.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

The constitution (v1.1.0) is written for one app. Depths is a second app, with its own protocol
and its own deployment tier. That is a scope change, not a violation, and it needs a **MINOR
amendment (→ 1.2.0) as the first task (T001)**, as spec 031 did for the hosted tier. The amendment:

- States that the principles apply to every app in the suite.
- Generalises Principle II to "each app owns one versioned, Rust-defined contract; shared domains
  (the container runtime) define their wire types once in a shared crate and each app's contract
  re-exports them".
- Adds a "Depths server (Linux container)" platform tier: Docker host, not a cluster; OIDC or
  GitHub sign-in required; holds Docker socket access, and must document a socket proxy.
- Lists the stack additions (CodeMirror, `notify`, OTel exporter) under Stack.

With that amendment in place:

| Principle | Verdict | How the design satisfies it |
|---|---|---|
| **I. Operator-first, explicit mutation** | **Pass, load-bearing** | <ul><li>Every lifecycle action, save and delete goes command → server-built preview (exact argv, cwd, env-file flags, what it reaches) → confirm with the preview id → run → audit.</li><li>The preview id is single-use and bound to the session and to the stack's content hash, so a preview made before someone else's edit cannot be confirmed (FR-019).</li><li>Free-text YAML is allowed for admins (FR-016), unlike 048. It is still previewable: the preview of a save is the diff of the file, plus `compose config` validation (spec Assumptions).</li><li>Nothing is optimistic: state changes only when docker reports it.</li><li>Docker's stderr is shown verbatim.</li></ul> |
| **II. One contract, backend owns the runtime** | **Pass (after T001)** | <ul><li>`crates/depths-protocol` is Depths' one contract, with ts-rs bindings generated into `apps/depths/web/src/protocol/generated/`, and a drift check is added to `scripts/ci.sh`.</li><li>The container wire types are defined once in `crates/container-runtime` and re-exported by both contracts.</li><li>The web app never touches Docker or the files.</li></ul> |
| **III. Tested at the boundaries** | **Pass** | <ul><li>Playwright E2E per story against a fake backend.</li><li>A round-trip test per protocol variant.</li><li>A failing-first unit test per rule: path confinement, symlink escape, conflict, role resolution, the one-shot status rule, one op per stack, preview binding.</li><li>Fathom's whole suite stays green on every extraction commit (FR-044, SC-007), and the bindings drift check proves the generated TS is byte-identical.</li></ul> |
| **IV. Density scales with width** | **Pass** | <ul><li>The stack list is virtualised.</li><li>Tabs collapse into a single pane with a tab strip on phones.</li><li>No horizontal scroll, and the log view wraps by default.</li><li>The theme and primitives are shared, so contrast tests (`color.ts`) cover Depths too.</li><li>WCAG 2.2 AA.</li></ul> |
| **V. Fast and quiet** | **Pass, one risk tracked** | <ul><li>With no session connected, nothing runs (R9).</li><li>Events and fs-watch are debounced into one inventory.</li><li>Stats run only while a stats tab is visible.</li><li>Log and shell buffers are bounded rings with a visible cap.</li><li>**Risk**: re-inventory spawns `docker ps`. This is mitigated by debouncing, and by one batched call per refresh rather than one per stack.</li></ul> |
| **VI. User data is sacred** | **Pass** | <ul><li>Writes are the operator's bytes, made atomically (temp file + rename in the same dir, mode and owner preserved).</li><li>A conflict check runs against the hash shown when editing began.</li><li>Delete keeps the directory unless the person chooses otherwise.</li><li>An unreadable roles file keeps the last good mapping, or, at first start, grants nothing, which is fail-closed.</li><li>The audit log is append-only.</li><li>Fathom's state files are untouched by the extraction: `container_adoption.json` keeps its path.</li></ul> |

**Hosted denials** (Fathom): unchanged. Fathom's hosted server still denies the container scene.
Depths is the supported way to manage Docker on a server.

## Project Structure

### Documentation (this feature)

```text
specs/050-depths-stack-manager/
├── plan.md              # This file
├── research.md          # Phase 0: Dockge behaviour, crate seams, auth, proxy, packaging
├── data-model.md        # Phase 1: entities, invariants, state transitions
├── quickstart.md        # Phase 1: how to validate, light fixtures
├── contracts/
│   ├── protocol.md      # Depths WS commands/events + HTTP endpoints
│   ├── config.md        # CLI flags, env vars, roles.toml, example compose
│   └── shared-crates.md # Public surface of the extracted crates and packages
├── checklists/
│   └── requirements.md
└── tasks.md             # Phase 2 (/speckit-tasks)
```

### Source Code (repository root)

```text
Cargo.toml                         # EDIT: workspace members += crates/*, apps/depths/server
pnpm-workspace.yaml                # NEW: ui, packages/*, apps/depths/web
package.json                       # NEW: root, private, packageManager pnpm@12.4.2
pnpm-lock.yaml                     # MOVED from ui/ (single root lockfile)

crates/
├── container-runtime/             # EXTRACTED from src/backend/container_runtime + kind.rs Runner
│   └── src/{lib,runner,stream,probe,inventory,grouping,compose,preview,blast_radius,
│            mutate,stats,logs,session,sink,adoption,wire}.rs
├── web-auth/                      # EXTRACTED from server/src/{session,csrf,oidc}.rs + http.rs parts
│   └── src/{lib,session,csrf,login,cookie,extract,provider,oidc,github}.rs
├── audit-log/                     # EXTRACTED from server/src/audit.rs, generic over the entry
│   └── src/lib.rs
├── depths-core/                   # NEW: domain (no I/O)
│   └── src/{lib,stack,status,discovery,name,role,plan,ports}.rs
├── depths-stacks-fs/              # NEW: StackStore adapter on the stacks dir
│   └── src/lib.rs
└── depths-protocol/               # NEW: the Depths contract, ts-rs
    ├── src/{lib,cmd,event,types}.rs
    └── tests/export_bindings.rs

apps/depths/
├── server/                        # NEW bin `depths`: Axum + Tower, clap, wiring only
│   └── src/{main,config,app,ws,http,ops,watch,telemetry}.rs
├── web/                           # NEW: React/Vite SPA (@companion/depths-web)
│   ├── src/{main.tsx,app/,features/{stacks,editor,tabs,audit,auth},protocol/}
│   ├── tests/{e2e/<story>/,unit/}
│   └── playwright.config.ts
├── Dockerfile                     # NEW: multi-stage, cargo-chef
└── compose.yaml                   # NEW: example with socket proxy, restart: ${RESTART_POLICY:-no}

packages/
├── theme/                         # EXTRACTED from ui/src/theme + lib/utils (@companion/theme)
├── ui/                            # EXTRACTED from ui/src/components/ui (@companion/ui)
└── terminal/                      # EXTRACTED + new: TerminalSurface, LogView, RingBuffer,
                                   #   PaneStates, YamlDiffView (@companion/terminal)

src/backend/container_runtime/     # SHRINKS to Fathom-specific: cluster_link, kind label post-pass,
                                   #   requirement(&BackendCmdV1), sink + persistence adapters
src/protocol/v1/container_runtime.rs  # becomes `pub use container_runtime::wire::*`
server/src/                        # session/csrf/oidc/audit become re-exports + thin adapters
ui/                                # imports @companion/*; behaviour unchanged

docs/guides/depths/                # NEW: install, sign-in (OIDC, GitHub), roles, socket proxy,
                                   #   migrating from Dockge
README.md                          # REBRAND: the orchestration companion suite
CONTRIBUTING.md                    # EDIT: workspace layout, crates/packages, Depths dev loop
```

**Structure Decision**:
- `apps/*` holds entry points only: the Depths server binary and web app.
- `crates/*` and `packages/*` hold the shared and domain code, per the user's direction and the
  global layout rules.
- Fathom's existing roots (`src/`, `server/`, `ui/`, `daemon/`) stay where they are. Moving them
  into `apps/` touches every Tauri, release and CI path, has no Depths benefit, and is left for a
  separate change.
- `depths-core` is the hexagonal core. Its ports are `StackStore` (the fs adapter) and
  `ContainerRuntime` (the adapter in `apps/depths/server` over `crates/container-runtime`).

## Phasing

| Phase | Content | Gate |
|---|---|---|
| A. Foundation | <ul><li>T001 constitution amendment.</li><li>Root pnpm workspace + lockfile move + ~15 path updates.</li><li>Empty crate/package skeletons in the workspaces.</li></ul> | `scripts/ci.sh` green; Fathom E2E green |
| B. Extraction (parallel) | <ul><li>B1 `container-runtime`.</li><li>B2 `web-auth` + `audit-log`.</li><li>B3 `@companion/theme`, `ui`, `terminal`.</li></ul> | Each sub-track lands green on its own; bindings drift zero |
| C. Depths backend | <ul><li>core.</li><li>fs adapter.</li><li>protocol.</li><li>server: auth, WS, ops, watch, audit.</li></ul> | Unit + round-trip tests |
| D. Depths web | <ul><li>Per story: US1 list → US2 lifecycle → US3 edit → US4 tabs → US5 roles/audit.</li></ul> | Playwright per story |
| E. Ship | <ul><li>Dockerfile, example compose, quickstart real-daemon run.</li><li>Guides, README rebrand, CONTRIBUTING.</li><li>Brochure screens last.</li></ul> | Quickstart passes; hygiene clean |

B1, B2 and B3 touch disjoint files and can run as parallel agents. C needs B1 and B2, and D needs
B3 and C's protocol.

## Post-design Constitution Re-check

The re-check was done after writing data-model.md and contracts/. It found no new violations.

- The preview binding (session, stack hash, single use) is in the protocol contract.
- The idle rule is in the server contract: watchers start on the first session and stop on the
  last.
- The fail-closed roles rule is in config.md.

The only gate is still T001.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| Six new crates (3 shared, 3 Depths) | FR-043 forbids copying. The hexagonal core needs domain separated from fs and Docker adapters | Depending on `fathom` pulls kube and the desktop backend into a Docker tool; one `depths` crate mixes domain and I/O and cannot be tested without both |
| Root pnpm workspace, lockfile moved | `packages/*` must be importable by `ui/` and `apps/depths/web` | Per-app lockfiles with `file:` links duplicate dependency trees and drift |
| CodeMirror 6 (new web dependency) | FR-017 needs highlighting and line-anchored errors | A textarea can't anchor errors; Monaco is several MB with workers |
| `notify` (new Rust dependency) | Edits made outside Depths must show up (FR-003) without polling while idle | Polling mtimes wakes up regularly with no one watching (Principle V) |
| A second contract (`depths-protocol`) | Depths is a different product with different commands | Adding Depths' commands to Fathom's `BackendCmdV1` couples two release lines and two UIs |

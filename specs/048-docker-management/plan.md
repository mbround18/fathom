# Implementation Plan: Docker and container-runtime management

**Branch**: `048-docker-management` | **Date**: 2026-10-02 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/048-docker-management/spec.md`

## Summary

A top-level Docker scene, present only when a container runtime is installed and running, that
lists containers grouped by project alongside images, volumes and networks, and acts on them under
the same guard the cluster side already uses. Two decisions shape the whole implementation: every
operation must be describable before it runs (so creation is form-driven and there is no free-text
command field), and a project is read-only until the operator adopts it.

The technical approach follows from the first of those. The backend reaches the runtime by invoking
its command-line interface through the existing mockable `Runner` trait in `src/backend/kind.rs`,
asking for machine-readable output (`--format '{{json .}}'`). That choice is not convenience: FR-018
requires the preview to be "a form the operator could reproduce themselves", and a literal command
line is exactly that, whereas a socket API call is not. It also makes every unit test a canned
string against a mock runner, needs no new dependency, and covers Docker-compatible alternative
runtimes without socket negotiation. See [research.md](./research.md) for the rejected alternative.

Adoption is the other spine. It is a persisted set of project names in `~/.fathom`, consulted by a
single gate that every mutating command passes through, and it reaches into blast radius: a
destructive preview must *exclude* objects belonging to unadopted projects and say that it did.

## Technical Context

**Language/Version**: Rust (backend, edition as per workspace), TypeScript 5 / React 19 (UI)

**Primary Dependencies**: existing only — `tokio` for the bounded probe and process supervision,
`serde` + `ts-rs` for the one contract, the existing `Runner` abstraction for process invocation,
the existing PTY and log-stream machinery from 002 (`src/backend/node_shell.rs`,
`src/backend/log_streams.rs`), the existing bounded-history approach from 036. **No new crate.**
`bollard` and other runtime-API clients are explicitly rejected in research.md.

**Storage**: `~/.fathom/container_adoption.json`, written through
`src/backend/persistence.rs::fathom_config_dir()` next to `integrations.json`. Adoption is the only
persisted state this feature owns; inventory is never cached to disk.

**Testing**: `cargo test` unit tests against a mock `Runner` with canned output (the pattern
`docker_stopped_podman_running_prefers_podman` in `src/backend/kind.rs` already establishes);
protocol round-trip tests per new variant in `src/protocol/v1/cmd.rs` and `event.rs`; Playwright
E2E per user story against the scripted fake backend; one small real-daemon integration layer in
the quickstart, fixtures capped at a single ~4 MB image (see Resource budget below).

**Target Platform**: Linux desktop (MVP). The scene is desktop-only and denied in hosted mode.

**Project Type**: Tauri desktop app — Rust backend owning all runtime access, React frontend
talking only through generated protocol bindings.

**Performance Goals**: operator input handled in under 100 ms with 500 containers (SC-012);
usage figures visible within the same latency order as cluster metrics (US6).

**Constraints**: under 2% of one core when idle with the scene closed (SC-011, Principle V) — which
for this feature means *zero* runtime invocations while the scene is not shown; bounded usage
history with a visible cap; no operator file ever rewritten (SC-013, Principle VI).

**Scale/Scope**: 7 user stories, 47 functional requirements, 15 success criteria. 500 containers as
the sizing target. One new protocol module, one new backend module tree, one new UI feature tree.

### Resource budget for development and testing

The target machine is shared with the operator's own work and with other agents, so this is a
constraint on the plan, not an afterthought:

- **Test fixtures use one image: `busybox` (~4 MB).** Its shell satisfies the session tests, so
  `alpine` is not needed and no larger image is to be introduced. The compose fixture is two
  `busybox` services running `sleep`, which is a few MB of disk and near-zero CPU and memory.
- **Almost nothing needs a real daemon.** Inventory, preview, blast radius, adoption and the
  mutation gate are all unit-tested against a mock `Runner` with canned JSON. The UI is tested
  against the scripted fake backend. Only the quickstart's integration checks touch a live daemon.
- **No kind cluster is created for this feature.** The cluster cross-link is tested against mock
  runner output carrying the cluster label, and against the fake backend in the UI. A live
  cross-link check belongs in the quickstart, run by a human when a cluster already happens to
  exist — never by spinning one up for the test.
- **`CARGO_BUILD_JOBS=4` by default, lower when the machine is in use.** `cargo check` in
  preference to `cargo build` during development.
- **Never prune, never remove an image, never stop a container that the test did not create.**
  The repository's own rule applies to its own test suite: other projects share this daemon. Every
  fixture is created with a recognisable name prefix and removed by name, never by prune.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Verdict | How this design satisfies it |
|---|---|---|
| **I. Operator-first, explicit mutation** | **Pass, and load-bearing** | The CLI-invocation approach exists because FR-018 demands a reproducible preview. Every mutating command carries a preview the operator sees first; nothing is optimistic — state changes only when the runtime reports; failures surface the runtime's own stderr verbatim. FR-019 forbids any path to an unpreviewable operation, which is why there is no free-text field. |
| **II. One contract, backend owns the cluster** | **Pass** | All runtime access lives in `src/backend/container_runtime/`. Types are defined once in `src/protocol/v1/container_runtime.rs` with `ts-rs`, bindings generated into `ui/src/protocol/generated/`, and `scripts/ci.sh`'s existing drift step fails the build if they go stale. The UI never shells out. Precedent: `src/protocol/v1/kind_cluster.rs` already puts a non-Kubernetes domain inside the one contract. |
| **III. Tested at the boundaries (NON-NEGOTIABLE)** | **Pass** | Each of the 7 stories gets E2E tests against the scripted fake backend. Each new `BackendCmdV1` and `StreamEvent` variant gets a round-trip test. Each behavioural rule — the adoption gate, the blast-radius exclusion, the fail-closed adoption record — gets a unit test that fails without it. |
| **IV. Density scales with width** | **Pass** | The scene is list-heavy: virtualized rows, mobile-first density, no horizontal scroll, WCAG 2.2 AA. Project groups collapse at narrow widths rather than overflowing. |
| **V. Fast and quiet** | **Pass, with one risk tracked** | No runtime invocation at all while the scene is closed (FR-045) — the probe result is already cached by the existing kind prerequisites path. Usage sampling is a discrete `stats --no-stream` tick, not a long-lived stream, runs only while the usage view is visible, and stops on navigation. Bounded histories with a visible cap (FR-038). Superseded request events discarded (FR-046). **Risk**: invoking the CLI per refresh is a process spawn; mitigated by one batched invocation per object class per refresh, operator-initiated or visibility-scoped, never a timer while hidden. |
| **VI. User data is sacred** | **Pass** | A project definition file is read and never written, moved or deleted (FR-033). An unreadable adoption record falls back to *everything unadopted* (FR-047) — fail closed, so corrupt state can never widen what Fathom may destroy. |

**Hosted mode**: the constitution's hosted section requires that desktop-only features not be
exposed. A new `HostedDenial::ContainerRuntime` variant is added rather than reusing
`HostedDenial::Kind`, whose documented meaning is local kind provisioning — a denial has to be
honest about *why*, and "a server has no local container runtime, and exposing the host's daemon
to signed-in users would grant privileges they do not have" is a different reason.

**No violations to track.** The Complexity Tracking table below is therefore empty by design.

## Project Structure

### Documentation (this feature)

```text
specs/048-docker-management/
├── plan.md              # This file
├── research.md          # Phase 0: the CLI-vs-API decision and the rest
├── data-model.md        # Phase 1: entities, invariants, state transitions
├── quickstart.md        # Phase 1: how to validate, including the light fixtures
├── contracts/
│   └── protocol.md      # Phase 1: commands, events and types this feature adds
├── checklists/
│   └── requirements.md  # Spec quality checklist (all green)
└── tasks.md             # Phase 2 output — NOT created by /speckit-plan
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── container_runtime.rs        # NEW — every type this feature adds, ts-rs derived
│   ├── cmd.rs                      # EDIT — new BackendCmdV1 variants
│   ├── event.rs                    # EDIT — new StreamEvent variants
│   ├── mutation.rs                 # EDIT — HostedDenial::ContainerRuntime + policy arms
│   └── mod.rs                      # EDIT — module registration and re-exports
├── backend/
│   ├── container_runtime/
│   │   ├── mod.rs                  # NEW — module surface
│   │   ├── probe.rs                # NEW — presence/reachability, reusing kind::detect_prerequisites
│   │   ├── inventory.rs            # NEW — containers, images, volumes, networks; JSON parsing
│   │   ├── grouping.rs             # NEW — project grouping and the no-project bucket
│   │   ├── adoption.rs             # NEW — the persisted set and the one mutation gate
│   │   ├── preview.rs              # NEW — the reproducible command line per operation
│   │   ├── blast_radius.rs         # NEW — what a destructive op reaches, and what it excludes
│   │   ├── mutate.rs               # NEW — issue, report per-object outcome, verify after
│   │   ├── stats.rs                # NEW — visibility-scoped sampling, bounded history
│   │   ├── logs.rs                 # NEW — adapter onto log_streams.rs
│   │   ├── session.rs              # NEW — adapter onto node_shell.rs PTY machinery
│   │   ├── compose.rs              # NEW — project definition file discovery, read-only
│   │   └── cluster_link.rs         # NEW — container ↔ local cluster, both directions
│   ├── cmd/container_runtime.rs    # NEW — command dispatch
│   ├── cmd.rs                      # EDIT — dispatch wiring
│   └── kind.rs                     # EDIT — expose Runner/SystemRunner for reuse; probe unchanged
└── ...

ui/src/
├── features/containers/            # NEW — the scene
│   ├── ContainersScene.tsx
│   ├── ContainerGroupList.tsx      # project groups, virtualized
│   ├── ObjectPanes.tsx             # images / volumes / networks
│   ├── AdoptionControl.tsx         # adopt, release, and the "not adopted" explanation
│   ├── OperationPreview.tsx        # the literal command, the removal list, the blast radius
│   ├── UsagePanel.tsx
│   └── ClusterLinkBadge.tsx
├── store/                          # EDIT — one slice for runtime state
├── layout/                         # EDIT — sidebar entry, conditional on presence
└── protocol/generated/             # GENERATED — do not hand-edit

ui/tests/
├── e2e/                            # NEW — one spec per user story, fake backend
└── support/                        # EDIT — fake backend handlers for the new commands
```

**Structure Decision**: the existing Tauri split, unchanged. The backend gains one module tree
under `src/backend/container_runtime/` rather than a flat file, because the spec's requirement
groups — adoption, preview, blast radius, mutation, inventory — are genuinely separate concerns and
the blast-radius logic in particular needs to be unit-testable in isolation. Mirrors how
`src/backend/mcp/`, `src/backend/oidc/` and `src/backend/agent/` are already organised. The UI
gains one feature tree under `ui/src/features/containers/`, alongside `daemon/` and `metrics/`.

## Complexity Tracking

> No constitution violations. Nothing to justify.

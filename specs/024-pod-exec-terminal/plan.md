# Implementation Plan: Pod Exec Session Gate (Confirmation + RBAC Pre-Check)

**Branch**: `024-pod-exec-terminal` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/024-pod-exec-terminal/spec.md`

## Summary

Fathom's existing in-app pod-exec terminal (spec 002) opens a shell session the instant a
container is chosen, with no confirmation and no proactive permission check — the Kubernetes
API server's own rejection is the only gate today. This spec adds an explicit confirmation
(naming namespace/pod/container) before any `StartPodExec` attach request is sent, and a
proactive `SelfSubjectAccessReview`-based permission pre-check that skips the confirmation
entirely (showing a permission-denied message instead) when the operator lacks `pods/exec`.
Technical approach: reclassify `StartPodExec` in `src/protocol/v1/mutation.rs` from
`ReadOnly` to `Namespaced` (reusing the existing mutation-guard mechanism, no new variant);
add one new read-only command/event pair, `CheckPodExecAccess`/`PodExecAccessChecked`; extend
`PodTerminalTab.tsx`'s phase state machine with `checking`/`denied`/`confirming` phases,
reusing the existing `ConfirmDialog` component. No new dependencies.

## Technical Context

**Language/Version**: Rust 2024 edition (backend), TypeScript/React (Tauri v2 UI, `ui/`)

**Primary Dependencies**: `kube` 2.0.1 (`ws` feature, already enabled) + `k8s-openapi`
(`authorization::v1::SelfSubjectAccessReview`, already used by `src/backend/rbac_access.rs`)
on the backend; `xterm`/`@xterm/addon-fit` (already in `ui/package.json`) and the existing
`ConfirmDialog` component on the frontend. No new dependency introduced.

**Storage**: N/A — no persisted state; the access-check result and confirmation state are
transient, in-memory only (component state / short-lived store slice), like the existing exec
session state.

**Testing**: `cargo test` (backend unit + protocol round-trip tests), `pnpm -C ui test`
(Vitest), `pnpm -C ui typecheck`, `pnpm -C ui lint`, Playwright E2E
(`ui/tests/e2e/pod-exec-gate.spec.ts`) against the fake backend.

**Target Platform**: Desktop (Tauri v2: Linux/Windows/macOS), same as the rest of Fathom.

**Project Type**: Desktop app — Rust backend (`src/`) + React/TS frontend (`ui/`), existing
single-repo layout; no new project/module boundary introduced.

**Performance Goals**: The permission pre-check must not add perceptible delay beyond one
extra network round-trip (`SelfSubjectAccessReview` is a single lightweight API call); no
change to existing streaming performance (Constitution V's 100ms interaction budget is
unaffected — this feature adds one gate before attach, not to the hot streaming path).

**Constraints**: Must not weaken the Kubernetes API server's own enforcement at attach time
(FR-007); must not change any existing pod-exec session behavior beyond adding the gate
(FR-008); the dev-bridge sandbox guard's existing contract (`tools/devbridge/src/guard.rs`)
must keep passing its existing tests with `StartPodExec` now `Namespaced`.

**Scale/Scope**: One new protocol command/event pair, one `mutation_target()` reclassification,
one UI component's state machine extended by three phases, one new Playwright spec. No new
backend module; the new logic lives in the existing `src/backend/exec.rs`.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design below.*

- **I. Operator-First, Explicit Mutation**: This spec exists specifically to bring
  `StartPodExec` into compliance with "every mutating action MUST pass through an explicit
  confirmation that names every affected object by namespace/name" — today it does not.
  **PASS** (this is the gap being closed).
- **II. One Contract, Backend Owns the Cluster**: The new `CheckPodExecAccess` command follows
  the existing UI-talks-only-through-`src/protocol/`-contract rule; the `SelfSubjectAccessReview`
  call happens entirely in the backend, exactly like every other cluster call. **PASS**.
- **III. Tested at the Boundaries**: New protocol variants get serde round-trip tests; the new
  backend logic gets unit tests; the new UI behavior gets a Playwright E2E spec against the
  fake backend, per this project's standard pattern (spec 002 already established this for
  the surrounding exec feature). **PASS** (see quickstart.md's "Automated coverage").
- **IV. Density Scales With Width**: No layout change — the confirmation is a modal dialog
  already responsive at all widths (`ConfirmDialog` is reused, not rebuilt). **PASS/N/A**.
- **V. Fast and Quiet**: One extra API call only when the operator actually opens the Terminal
  tab for a specific pod/container (not on every render/poll); no continuous polling added.
  **PASS**.

No violations to justify; Complexity Tracking table is empty/omitted.

## Project Structure

### Documentation (this feature)

```text
specs/024-pod-exec-terminal/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── protocol.md      # Phase 1 output
└── tasks.md              # Phase 2 output (/speckit-tasks — not yet created)
```

### Source Code (repository root)

This feature modifies existing files only; no new top-level modules.

```text
src/
├── protocol/v1/
│   ├── cmd.rs            # + CheckPodExecAccess variant, + round-trip test
│   ├── event.rs          # + PodExecAccessChecked variant, + round-trip test
│   └── mutation.rs       # StartPodExec: ReadOnly -> Namespaced; + classification test
├── backend/
│   ├── exec.rs           # + check_access() (SelfSubjectAccessReview), + unit tests
│   └── cmd/workloads.rs  # + dispatch CheckPodExecAccess -> exec::check_access
└── (tools/devbridge/src/guard.rs — no code change; existing tests re-verified against the
   reclassified StartPodExec)

ui/
├── src/
│   ├── store/
│   │   ├── index.ts              # + exec-access-check slice/types
│   │   └── handlers/stream.ts    # + PodExecAccessChecked handling (or a new handler file
│   │                               #   if stream.ts is scoped to stream-only events)
│   └── components/panels/
│       └── PodTerminalTab.tsx    # + checking/denied/confirming phases, + ConfirmDialog usage
└── tests/e2e/
    └── pod-exec-gate.spec.ts     # new: happy path + RBAC gate (2 scenarios)
```

**Structure Decision**: Pure extension of the existing single-repo Rust-backend +
React-frontend layout used by every prior spec (001-015); no new directories beyond the
feature's own `specs/024-pod-exec-terminal/`.

# Implementation Plan: In-App Local Kind Cluster Provisioning

**Branch**: `030-kind-cluster-provisioning` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/030-kind-cluster-provisioning/spec.md`

## Summary

Add an in-app way to detect, list, create, and delete local `kind` (Kubernetes-in-Docker)
clusters entirely from the Fathom UI, with no terminal required. The backend gains new
`BackendCmdV1` commands to probe host prerequisites (kind binary + container runtime), list
kind clusters, create a cluster (long-running, streamed progress, request_id-keyed — following
the existing `DrainNode`/`DrainProgress` pattern), and delete a cluster (destructive,
`ConfirmDialog`-gated like Helm uninstall / node drain). On successful creation, the new
cluster's kubeconfig context is merged into the user's kubeconfig and picked up by the existing
`RefreshContexts` / `FederationContextSwitcher` mechanism from spec 012 — no new/parallel
context registry is introduced. All host process invocation uses `tokio::process::Command`
with an argument array (never a shell string), and all user-supplied identifiers (cluster name)
are validated as DNS-1123-label-safe before ever reaching a subprocess argument.

## Technical Context

**Language/Version**: Rust (edition 2024, matches repo) for backend; TypeScript (strict) + React for frontend.

**Primary Dependencies**: `tokio` (process spawning, already used), `kube`/`kube-rs` config crate types for kubeconfig read/merge (already a dependency per spec 012's `src/backend/kube.rs`), `ts-rs` for generated protocol bindings (existing convention), shadcn/ui components (`ConfirmDialog`, dialog/form primitives) on the frontend.

**Storage**: The user's on-disk kubeconfig file (read via existing `load_kubeconfig*` helpers in `src/backend/kube.rs`, written via a new merge routine using the `kube::config::Kubeconfig` struct's serialize path — no new Fathom-owned storage format is introduced for cluster identity itself). No new entries under `~/.fathom/` are required beyond, optionally, remembering the last-used profile choice as a UI convenience (out of scope for v1 unless trivial).

**Testing**: `cargo test` for backend unit tests (name validation, mutation classification, kubeconfig merge on a fixture file) and protocol round-trip (serde) tests; Playwright for new e2e specs under `ui/tests/e2e/`, following existing scripted-fake-backend conventions (Principle III).

**Target Platform**: Same Tier 1 platforms as the rest of Fathom (Ubuntu 26.04, Windows 11, macOS 26) — `kind`/Docker/Podman availability varies by platform but detection code is platform-agnostic (binary lookup on `PATH` + a lightweight runtime ping).

**Project Type**: Existing desktop app (Tauri v2 backend + React frontend) — this feature is additive within that single project, not a new project.

**Performance Goals**: Prerequisite detection and cluster listing must return within ~1s under normal conditions (simple subprocess calls). Create/delete operations are inherently host-bound (30-90s) but must emit a progress event at least every few seconds (SC-004), matching the `DrainProgress` cadence.

**Constraints**: Frontend never spawns processes or touches the container runtime/kind binary directly (Principle II) — all binary invocation lives in the Rust backend behind the protocol. Every mutating command (`CreateKindCluster`, `DeleteKindCluster`) is confirmation-gated client-side via `ConfirmDialog` before being sent, matching the existing Helm/drain pattern. Cluster name input is passed only as a `Command` argument, never interpolated into a shell string.

**Scale/Scope**: Local development scope — expect a handful of kind clusters on one machine at a time, not a fleet. No pagination or large-list virtualization concerns beyond what the existing list components already provide.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Principle I (Operator-First, Explicit Mutation)**: PASS. Create and delete both require an explicit `ConfirmDialog` confirmation naming the cluster (and, for create, showing the profile/what-will-be-created summary) before any command is sent, mirroring `NodeActionsPanel.tsx` / `HelmReleaseActionsPanel.tsx`. No optimistic UI changes: the cluster list updates only after `CommandFinished`/`ProvisionFinished`-style terminal events. Backend errors (e.g. `kind` stderr) are surfaced verbatim.
- **Principle II (One Contract, Backend Owns the Cluster)**: PASS. All `kind`/container-runtime process invocation and kubeconfig file access live in the Rust backend (`src/backend/`), reached only via new `BackendCmdV1` variants over the existing protocol. The UI never shells out or reads `~/.kube/config` itself.
- **Principle III (Tested at the Boundaries)**: PASS, planned. New protocol variants get serde round-trip tests; name validation and kubeconfig-merge logic get Rust unit tests on fixtures; new user stories get Playwright e2e specs against the scripted fake backend, per existing `ui/tests/e2e/` conventions.
- **Principle IV/V (Density/Fast & Quiet)**: PASS — this is a small settings-style panel, not a high-volume list; no continuous polling beyond an on-demand refresh and the bounded lifetime of a single create/delete progress stream.
- **Principle VI (User Data Is Sacred)**: PASS with an explicit constraint — merging a new context into the user's kubeconfig MUST preserve every existing entry byte-for-byte other than the addition, and MUST never overwrite or reorder unrelated contexts/clusters/users. This is called out as a design requirement in data-model.md and must have a dedicated fixture-based unit test (merge onto a kubeconfig containing unrelated contexts must leave them untouched).

No violations requiring Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/030-kind-cluster-provisioning/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/
│   └── kind-cluster-provisioning.md
└── tasks.md              # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/backend/
├── kind.rs                  # NEW: prerequisite detection, list/create/delete kind clusters,
│                             #      kubeconfig merge-in of the new context
├── kube.rs                  # existing kubeconfig read helpers reused as-is
├── cmd/
│   └── cluster.rs           # extend with handlers for the new BackendCmdV1 variants
src/protocol/v1/
├── cmd.rs                   # add BackendCmdV1::{RefreshKindPrerequisites, RefreshKindClusters,
│                             #   CreateKindCluster, CancelCreateKindCluster, DeleteKindCluster}
├── event.rs                 # add StreamEvent::{KindProvisionProgress, KindClusterActionFinished}
│                             #   and a KindPrerequisites/KindClusterSummary payload type
├── mutation.rs               # classify new commands (Local for prereq/list/create/delete —
│                             #   they mutate Fathom's kubeconfig-context registry, not cluster objects)

ui/src/
├── components/panels/
│   └── KindClusterPanel.tsx  # NEW: prerequisites banner, cluster list, create/delete flows,
│                             #   reusing ConfirmDialog and a DrainProgress-style progress view
├── store/handlers/
│   └── stream.ts             # extend reducer for new StreamEvent variants
├── store/index.ts            # extend session/local state shape for kind cluster + prereq state
ui/tests/e2e/
└── kind-cluster-provisioning.spec.ts  # NEW: prereq empty-state, create confirm+progress,
                                        #      delete confirm, appears in context switcher
```

**Structure Decision**: Single-project desktop app structure (existing `src/` Rust backend +
`ui/` React frontend). No new top-level project. The feature is additive: one new backend
module (`src/backend/kind.rs`), extensions to the existing protocol files, one new frontend
panel component, and one new e2e spec — following the same shape as prior features (e.g. how
spec 012's federation switcher and spec 008's node actions are organized).

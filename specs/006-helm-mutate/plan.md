# Implementation Plan: Helm Mutate (Upgrade, Rollback, Uninstall) with GitOps-Ownership Warnings

**Branch**: `006-helm-mutate` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/006-helm-mutate/spec.md`

## Summary

Add upgrade, rollback, and uninstall to the read-only Helm Releases view shipped in 005. Rollback and uninstall need no new rendering: rollback replays an already-stored revision manifest (verified verbatim in `HelmRevisionSummary.manifest`), and uninstall removes release-storage records for Kubernetes to garbage-collect via owner references. Upgrade is the one genuinely new capability — since no Go-template-rendering crate exists in this Rust codebase (verified), it shells out to the operator's local `helm` binary (`helm upgrade --dry-run` for preview, `helm upgrade`/`helm uninstall` for apply) to get exact Helm semantics rather than reimplementing chart rendering. All three actions reuse 003's diff-preview UI shape, `ConfirmDialog`'s existing named-item confirmation, and 005's `GitOpsOriginRef`-based ownership detection for the "may be reverted by GitOps" warning — no new detection path, no new confirmation mechanism.

## Technical Context

**Language/Version**: Rust (edition 2024, workspace standard) for the backend/protocol; TypeScript (strict) for the frontend.

**Primary Dependencies**: `kube-rs` (existing, release-storage Secret/ConfigMap reads/writes), `tokio` (existing). New: shells out to the operator's local `helm` binary via `std::process::Command`/`tokio::process::Command` for upgrade preview/apply and uninstall (R4 in research.md) — no new Cargo dependency, an external-binary dependency instead, detected at call time via `which helm`/`helm version`.

**Storage**: Kubernetes API — Helm's own release-storage Secret/ConfigMap objects (read via existing 005 code, written via `helm` CLI invocations for upgrade/uninstall, and via the same release-storage read/write path for rollback since no chart rendering is needed there).

**Testing**: `cargo test` (protocol round-trip + unit tests against fixture `HelmReleaseDetails`/revision data, no live cluster required for rollback/failure-path logic), Vitest (frontend unit), Playwright against a scripted fake backend (E2E per user story) — per Constitution Principle III, matching 003/005's test conventions.

**Target Platform**: Tauri v2 desktop app — Ubuntu 26.04, Windows 11, macOS 26 (constitution Tier 1).

**Project Type**: Desktop app (existing Rust backend + React/TS frontend, single repo).

**Performance Goals**: SC-001 — rollback from target-selection to live in under 30s of interaction (excluding cluster apply latency), consistent with existing mutate-action UX in the app.

**Constraints**: Constitution Principle V (Fast and Quiet) — diff-preview computation for rollback (no rendering needed) must not block the UI thread; upgrade's `helm upgrade --dry-run` shellout runs as an async backend task, never synchronously blocking event handling.

**Scale/Scope**: Single-release mutate actions only (explicit non-goal: multi-release bulk operations, FR-013). Scope matches 005's release list size assumptions (no new list-scale concerns).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design below.*

- **Principle I (Operator-First, Explicit Mutation)**: PASS. Every mutating action (rollback, upgrade, uninstall) requires explicit confirmation naming the affected release by namespace/name (`ConfirmDialog`, reused as-is — R3), with diff-preview before apply for rollback/upgrade (FR-002/FR-004) and no optimistic UI changes (state only updates after `HelmMutationApplied`).
- **Principle II (One Contract, Backend Owns the Cluster)**: PASS. All release-storage reads/writes and the `helm` CLI invocations happen exclusively in the Rust backend; the frontend only sends the 5 new `BackendCmdV1` variants and receives the 3 new `SystemEvent` variants (contracts/protocol.md) — never raw release-storage bytes, never talks to the cluster or invokes `helm` itself (FR-011).
- **Principle III (Tested at the Boundaries)**: PASS, planned. Round-trip serde tests for all 5 new commands + 3 new events (tasks.md Foundational phase); unit tests for rollback diff/stale-revision logic against fixtures; scripted-fake-backend E2E per user story (quickstart.md's Automated Coverage section).
- **Principle IV (Density Scales With Width)**: N/A — reuses existing Inspector/dialog/diff-view layout patterns, no new pane types.
- **Principle V (Fast and Quiet)**: PASS. `helm upgrade --dry-run` shellout is async and bounded to the upgrade-preview action only; rollback/uninstall never invoke a subprocess for rendering (rollback needs none; uninstall's `helm uninstall` is a one-shot mutation, not a hot path).
- **Principle VI (User Data Is Sacred)**: N/A — no `~/.fathom/` config changes in this feature.
- **Dependencies SHOULD be stable releases** (Development Workflow & Quality Gates): the `helm` binary dependency is external/detected-at-runtime, not a new Cargo dependency, so this gate doesn't directly apply; it is still flagged below in Complexity Tracking since it's a deviation from 005's "no shellout" read-path precedent.

**Result**: No violations requiring justification beyond the one documented exception below (helm CLI shellout for mutation only, not reads) — recorded in Complexity Tracking per the constitution's own instruction that "any violation MUST be justified in the plan's Complexity Tracking table," since this is a precedent deviation worth an explicit record even though it does not violate a MUST.

## Project Structure

### Documentation (this feature)

```text
specs/006-helm-mutate/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/
│   └── protocol.md       # Phase 1 output
└── tasks.md              # Phase 2 output (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── cmd.rs           # + 5 BackendCmdV1 variants (PreviewHelmRollback, ApplyHelmRollback,
│   │                    #   PreviewHelmUpgrade, ApplyHelmUpgrade, UninstallHelmRelease)
│   ├── event.rs         # + 3 SystemEvent variants (HelmMutationPreviewReady/Applied/Failed)
│   ├── mutation.rs       # + mutation_target() arms for the 5 new commands (Namespaced)
│   └── helm.rs           # + HelmMutationAction/Stage, HelmMutationPreviewReady/Applied/Failed structs
├── backend/
│   ├── helm.rs            # + rollback (release-storage read/write, no rendering),
│   │                       #   upgrade (helm CLI shellout: dry-run + apply), uninstall (helm CLI)
│   └── cmd/
│       └── helm.rs         # (new or extends existing dispatch) command handlers for the 5 new variants

ui/src/
├── resources/
│   └── helm.tsx           # + Rollback/Upgrade/Uninstall action entry points in the Inspector
├── components/
│   ├── ConfirmDialog.tsx   # reused unmodified
│   └── panels/             # generalized diff-view component (extends 003's YAML-apply diff modal
│                            #   to accept two arbitrary manifest strings)
└── store/
    └── editSession.ts      # pattern reference only; Helm mutate gets its own small preview-state
                             #   slice mirroring this shape, not a shared module (different domain)

tests/
└── protocol_serde.rs       # + roundtrip() tests for all 5 commands + 3 events
```

**Structure Decision**: No new top-level project/module — this feature extends the existing `src/protocol/v1/helm.rs`, `src/backend/helm.rs`, and `ui/src/resources/helm.tsx` modules 005 already established, following the same "extend, don't fork" approach 005 used on top of 004's `related.rs`. The one new architectural element (shelling out to `helm` for upgrade/uninstall) lives entirely inside `src/backend/helm.rs`, behind the same command-handler boundary as the rest of the release-storage logic — no new top-level backend module is warranted for three functions.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|---------------------------------------|
| Shelling out to the operator's local `helm` binary for upgrade preview/apply and uninstall, deviating from 005's "no `helm` binary shellout" read-path precedent | Upgrade requires real Go-template chart rendering (hooks, subchart/dependency resolution, `.Files`, template functions) to produce a preview/apply that actually matches what Helm would do (FR-004); no Rust crate for this exists today (verified, research.md R4) | Reimplementing a Go-template-compatible rendering engine in Rust: months of work, high risk of subtly wrong output for real-world charts, directly undermines FR-004's promise that the diff-preview shows what upgrade "would produce." A values-only manifest patch without re-templating was also rejected — it silently diverges from real Helm semantics whenever a value is referenced inside a template conditional/loop, which is common. |

# Implementation Plan: Container Image Inspection

**Branch**: `010-container-image-inspection` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/010-container-image-inspection/spec.md`

## Summary

Add a per-container "Image" tab to the Pod inspector that, on demand, fetches the OCI manifest and
config (never layer blobs) for a container's image, using anonymous access with a fallback to the
Pod's own `imagePullSecrets`, and displays registry/repo/digest, total size, layer list with
build-history commands, and a best-effort base-image guess. Read-only, backend-only registry
access via the new `oci-client` crate (R1).

## Technical Context

**Language/Version**: Rust (edition 2024, existing workspace) + TypeScript (strict, existing `ui/`)

**Primary Dependencies**: `oci-client` 0.18 (new), reusing existing `kube`, `tokio`, `serde`,
`ts-rs`; frontend reuses existing Zustand store, shadcn/ui Tabs, existing `protocol/commands` glue.

**Storage**: In-memory only (session-scoped inspection cache, keyed by manifest digest); nothing
persisted to `~/.fathom/`.

**Testing**: `cargo test` (protocol round-trip + parsing/auth-fallback unit tests against fixture
bytes/mock HTTP), Vitest component tests, Playwright e2e (`ui/tests/e2e/us1-image-inspection-*.spec.ts`).

**Target Platform**: Same as rest of app — Tier 1 desktop (Ubuntu/Windows/macOS).

**Project Type**: Desktop app (Tauri v2 + Rust backend), existing structure.

**Performance Goals**: SC-001 — image data visible within ~5s of tab open (network permitting).

**Constraints**: No layer blob downloads ever (FR-002/FR-013); no eager/background fetch
(FR-003); no new credential UI (FR-006); no CVE scanning (FR-012); no interactive/OIDC auth
(FR-014); read-only (FR-015).

**Scale/Scope**: One new protocol command/event pair, one new backend module, one new frontend
panel + store slice, additive to the existing Pod inspector.

## Constitution Check

- **I. Operator-First, Explicit Mutation**: This feature performs zero mutations (confirmed:
  only `Api<Pod>::get` and `Api<Secret>::get` reads, plus outbound registry `GET`s for
  manifest/config; no `patch`/`put`/`delete` anywhere in scope). PASS — no confirmation dialog
  needed since nothing mutates.
- **II. One Contract, Backend Owns the Cluster**: All registry HTTP calls happen in the Rust
  backend via `oci-client`; the UI only sends `InspectContainerImage` and renders the resulting
  event. PASS.
- **III. Tested at the Boundaries**: New command/event get round-trip serde tests; parsing +
  auth-fallback get unit tests against fixtures; UI gets component + e2e tests per user story.
  PASS (see Testing above).
- **Density/Fast&Quiet/User Data**: No layout regression (additive tab); fetch is on-demand only
  so no idle-CPU or eager-network impact; nothing written to `~/.fathom/`. PASS.

No violations requiring Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/010-container-image-inspection/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/image-inspection.md
└── tasks.md
```

### Source Code (repository root)

```text
Cargo.toml                          # + oci-client dependency
src/protocol/v1/images.rs           # new: ImageRef, ImageLayer, ImageInspection, ImageInspectError
src/protocol/v1/cmd.rs              # + BackendCmdV1::InspectContainerImage
src/protocol/v1/event.rs            # + ResourceReadEvent::ContainerImageInspected{,Failed}
src/protocol/v1/mod.rs              # re-exports
src/backend/images.rs               # new: registry fetch, auth fallback, cache, base-image heuristic
src/backend/mod.rs                  # dispatch InspectContainerImage -> images::inspect_container_image
src/backend/state.rs                # + image_inspect_cache: HashMap<String, ImageInspection>
tests/protocol_serde.rs             # + round-trip tests for new types
src/backend/images.rs (tests mod)   # unit tests: manifest/config parsing, auth fallback, base-image match

ui/src/protocol/types.ts            # regenerated via ts-rs / existing bindings script
ui/src/components/panels/ImageInspectPanel.tsx   # new
ui/src/components/Inspector.tsx     # + "Image" tab for Pod
ui/src/store/*                      # + imageInspections slice/cache
ui/src/components/panels/ImageInspectPanel.test.tsx  # new (vitest)
ui/tests/e2e/us1-image-inspection-public.spec.ts      # new
ui/tests/e2e/us1-image-inspection-private-auth.spec.ts # new (US2)
ui/tests/e2e/us1-image-inspection-base-image.spec.ts   # new (US3), or folded into public spec
```

## Complexity Tracking

None — no constitution gate violated.

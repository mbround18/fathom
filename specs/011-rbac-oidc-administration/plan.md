# Implementation Plan: Cluster-Admin RBAC / OIDC-OAuth Administration

**Branch**: `011-rbac-oidc-administration` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/011-rbac-oidc-administration/spec.md`

## Summary

Fathom already reads Role/ClusterRole/RoleBinding/ClusterRoleBinding (list + detail + delete)
through the existing `BackendCmdV1`/`BackendEventV1` protocol and the generic
`ApplyResourceYaml` diff-preview/apply/conflict-detection command from 003. This feature:

1. Adds React list/detail views for the 4 existing RBAC read sections (currently un-descriptored,
   falling back to a generic view) with readable rule rendering and dangling-`roleRef` badges.
2. Wires RoleBinding/ClusterRoleBinding create + edit to the **existing** `ApplyResourceYaml`
   command and `YamlDiffView` + `ConfirmDialog` components — no new mutation command needed for
   the write path itself, since 003's generic apply already handles arbitrary-kind create/update
   with resourceVersion-conflict detection.
3. Adds one new **read-only** protocol command pair for a live self-permission check
   (`RefreshRbacSelfAccess` / `RbacSelfAccessUpdated`) using `SelfSubjectAccessReview` /
   `SelfSubjectRulesReview`, to gate create/edit/delete actions per FR-007.
4. Adds one new **read-only** protocol command pair for namespace-access resolution
   (`ResolveNamespaceAccess` / `NamespaceAccessResolved`) computed entirely from already-fetched
   Role/ClusterRole/RoleBinding/ClusterRoleBinding objects — no new K8s API calls beyond listing
   those 4 kinds (which the existing Refresh* commands already do).
5. Adds a **frontend-only** OIDC configuration generator (pure computation from operator input,
   no cluster write) plus one new **read-only** protocol command to best-effort detect a
   kubeadm-style cluster (`kube-system/kubeadm-config` ConfigMap presence) for FR-009 acceptance
   scenario 1.
6. Existing `DeleteRoleBinding`/`DeleteClusterRoleBinding` commands are reused unchanged for the
   delete path (already namespaced/cluster-scoped classified in `mutation.rs`); only the frontend
   confirm-dialog wiring is new.

## Technical Context

**Language/Version**: Rust (edition 2024, workspace `cargo` toolchain already pinned); TypeScript
strict (existing `ui/` Vite/React 18 app)

**Primary Dependencies**: `kube` 2.0.1 (`runtime`, `derive`, `ws`), `k8s-openapi` 0.26.1
(`v1_34` — includes `authorization.k8s.io/v1` `SelfSubjectAccessReview`/`SelfSubjectRulesReview`
types already), `schemars`, `serde`/`serde_json`/`serde_yaml`, `ts-rs` for generated bindings.
Frontend: existing `diff` package (YamlDiffView), Vitest, Playwright. **No new dependencies
required** — `k8s-openapi` v1_34 already vendors the `authorization.k8s.io/v1` review types used
for the self-access-check and the generic `DynamicObject`/discovery machinery `apply.rs` already
uses covers RoleBinding/ClusterRoleBinding creation with no new client code paths.

**Storage**: N/A (no new on-disk state; OIDC generator output is ephemeral/exportable text only,
never persisted to `~/.fathom/`).

**Testing**: `cargo test` (protocol round-trip in `tests/protocol_serde.rs`, backend unit tests
for namespace-access resolution and self-access-review mapping using fixture
Role/ClusterRole/RoleBinding/ClusterRoleBinding data — no live cluster), `pnpm -C ui test`
(Vitest component tests), Playwright e2e under `ui/tests/e2e/us1-rbac-*.spec.ts` style files
following `us1-apply-yaml.spec.ts`'s fixture/mocked-backend convention.

**Target Platform**: Same Tauri v2 desktop app, Tier 1 platforms per constitution.

**Project Type**: Desktop app (Rust backend + React/TS frontend), existing structure.

**Performance Goals**: Namespace-access resolution must complete synchronously against
already-cached list data (no new cluster round trips beyond the 4 existing Refresh* reads);
must stay responsive per Principle V (no new watches/streams needed).

**Constraints**: No live mutation of kube-apiserver OIDC flags (FR-008, permanent scoping
decision — not a placeholder). Reuse 003's diff-preview/confirm/stale-conflict pattern exactly;
do not invent a second mutation pathway for RoleBinding/ClusterRoleBinding.

**Scale/Scope**: New Rust: 1 new backend module (`src/backend/rbac_access.rs`) for
self-access-review + namespace-access resolution + kubeadm-detection; ~6 new protocol
command/event variants; frontend: RBAC list/detail descriptors for 4 existing sections, a
RoleBinding/ClusterRoleBinding create/edit form that builds YAML for `ApplyResourceYaml`, a
namespace-access lookup view, an OIDC generator view, nav entries.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. All RoleBinding/ClusterRoleBinding
  create/edit/delete reuse the existing diff-preview (create/edit via `ApplyResourceYaml` +
  `YamlDiffView`) and named-object `ConfirmDialog` (delete, reusing existing
  `DeleteRoleBinding`/`DeleteClusterRoleBinding`) patterns verbatim. No optimistic UI updates —
  list refresh only after backend confirms (existing `refresh_*` re-run on write success is
  already the pattern). Backend errors (including API-server RBAC escalation rejections) surface
  verbatim via the existing generic error-notification path, unchanged.
- **II. One Contract, Backend Owns the Cluster**: PASS. All new reads (self-access-review,
  namespace-access resolution, kubeadm detection) are new `BackendCmdV1`/`BackendEventV1`
  variants with `ts-rs`-generated bindings, same as every existing command. The OIDC generator
  does no cluster I/O at all (pure frontend computation from operator input), consistent with
  FR-008/FR-010.
  All new commands are `MutationTarget::ReadOnly` (see contracts/protocol.md); the exhaustive
  match in `mutation.rs` forces this decision to be made explicitly at compile time.

**Post-Phase-1 re-check**: PASS, unchanged — data-model.md and contracts/protocol.md confirm no
new mutation pathway was introduced (RoleBinding/ClusterRoleBinding writes still go through
`ApplyResourceYaml`/`Delete*`), and no new persisted state or dependency was added.
- **III. Tested at the Boundaries**: PASS (planned). New protocol variants get round-trip tests
  in `tests/protocol_serde.rs` plus exhaustive-match updates; namespace-access resolution and
  self-access-review mapping get Rust unit tests against fixture RBAC objects; new frontend logic
  (dangling-roleRef detection, namespace-access aggregation rendering, OIDC flag generation) gets
  Vitest unit tests; each user story gets a scripted-fake-backend Playwright e2e spec named
  `us{N}-rbac-*.spec.ts`.
- **IV. Density Scales With Width**: PASS (planned) — new views follow existing list/detail pane
  conventions and responsive column-priority patterns already used by other Access Control
  sections; no new patterns needed.
- **V. Fast and Quiet**: PASS — namespace-access resolution runs against already-fetched/cached
  list data, no new polling/watches.
- **VI. User Data Is Sacred**: PASS — no new `~/.fathom/` persisted state; OIDC generator output
  is export-on-demand only, never auto-saved; Fathom's own kubeconfig-context/RBAC is respected
  unchanged (no privilege escalation attempted or assumed).

No violations; Complexity Tracking table not needed.

## Project Structure

### Documentation (this feature)

```text
specs/011-rbac-oidc-administration/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md         # Phase 1 output
├── contracts/            # Phase 1 output
└── tasks.md               # Phase 2 output (speckit-tasks)
```

### Source Code (repository root)

```text
src/backend/
├── rbac_access.rs        # NEW: self-access-review, namespace-access resolution, kubeadm detect
├── roles.rs               # existing (unchanged read/delete)
├── clusterroles.rs        # existing (unchanged)
├── rolebindings.rs        # existing (unchanged)
├── clusterrolebindings.rs # existing (unchanged)
└── cmd.rs                 # add dispatch arms for new read commands

src/protocol/v1/
├── cmd.rs                 # add RefreshRbacSelfAccess, ResolveNamespaceAccess, RefreshKubeadmDetection
├── event.rs                # add corresponding *Updated/*Resolved events + new Details types
└── mutation.rs             # new commands classified ReadOnly (see arm additions)

tests/protocol_serde.rs     # new sample_*/all_* entries for the above

ui/src/
├── resources/
│   ├── index.ts             # register descriptors for access/roles, access/rolebindings,
│   │                          access/clusterroles, access/clusterrolebindings
│   ├── roles.tsx             # NEW list/detail descriptor (readable rule table)
│   ├── clusterroles.tsx      # NEW
│   ├── rolebindings.tsx      # NEW (+ create/edit form building ApplyResourceYaml YAML)
│   ├── clusterrolebindings.tsx # NEW
│   ├── namespaceAccess.tsx   # NEW lookup view (Story 4)
│   ├── oidcConfig.tsx        # NEW generator view (Story 5, no protocol write)
│   └── nav.ts                 # add 2 new Access Control entries
├── components/rbac/           # NEW shared components (subject editor, rule table, badge)
└── store/                      # extend as needed for new command/event wiring

ui/tests/e2e/
├── us1-rbac-browse-roles.spec.ts
├── us2-rbac-create-binding.spec.ts
├── us3-rbac-edit-delete-binding.spec.ts
├── us4-rbac-namespace-access.spec.ts
└── us5-rbac-oidc-generator.spec.ts
```

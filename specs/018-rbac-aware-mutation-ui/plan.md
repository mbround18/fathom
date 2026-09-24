# Implementation Plan: Proactive RBAC-Aware Mutation UI

**Branch**: `018-rbac-aware-mutation-ui` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/018-rbac-aware-mutation-ui/spec.md`

## Summary

Spec 011 already introduced two RBAC-read mechanisms: a pure namespace-access *enumerator*
(`resolve_namespace_access()` in `src/backend/rbac_access.rs`, "what can subject X do, listed
out") and a point-lookup *self-access-review* (`RefreshRbacSelfAccess` → `RbacSelfAccessUpdated`,
backed by a live `SelfSubjectAccessReview` per check — "can I, right now, do verb V on resource R
in namespace N"). The point-lookup is the one that already matches this feature's required
semantics exactly (the spec's own wording, "SelfSubjectAccessReview-equivalent semantics", is
literally what it does), and it is already wired to gate exactly one surface today:
`RoleBindingForm.tsx`'s create/edit review button. This plan **generalizes that existing
point-lookup mechanism into a shared, reusable frontend hook and wires it to every other named
mutation surface**, rather than building a second authorization model on top of the enumerator.

Concretely:

1. **Frontend**: promote `RoleBindingForm.tsx`'s inline "dispatch a check, find the matching
   result, gate a boolean" pattern into a shared hook (`useMutationAccess`) plus a shared
   disabled-with-tooltip button affordance, and fix the `rbacAccess.selfAccess` store slice (today
   overwritten wholesale on every reply) to merge results by `(verb, resource, namespace)` so
   concurrent panels don't clobber each other's checks.
2. **Backend, one small addition**: `RefreshRbacSelfAccess` already handles the three surfaces
   with a static, well-known resource name (`nodes` for node actions, `secrets` for Helm's
   release-storage-based FR-008 mapping, `rolebindings`/`clusterrolebindings` for RBAC admin) with
   zero backend changes. The YAML apply/create flow is the one case where the target resource's
   real plural name is not statically known in the frontend — `src/backend/apply.rs` already
   resolves it per-apply via `kube::discovery::oneshot::pinned_kind`. A new command,
   `CheckResourceMutationAccess`, reuses that same discovery call to resolve the plural/group and
   then runs the identical `SelfSubjectAccessReview` point-check `handle_refresh_rbac_self_access`
   already performs, so YAML apply/create gets a correctly-scoped check instead of a guessed
   plural.
3. **RBAC admin's own controls**: `RoleBindingForm.tsx` is refactored onto the new shared hook
   (no behavior change), and RoleBinding/ClusterRoleBinding **delete** — ungated today — gets the
   same gating via a small optional `accessCheck` field added to `ActionDescriptor`.

## Technical Context

**Language/Version**: Rust (edition 2024); TypeScript strict (existing `ui/` Vite/React 18 app).

**Primary Dependencies**: `kube` (existing `discovery::oneshot::pinned_kind`, already used by
`apply.rs`), `k8s-openapi` (`authorization.k8s.io/v1` `SelfSubjectAccessReview`, already used by
`rbac_access.rs`). Frontend: existing Zustand store, shadcn `Tooltip` component (already present,
unused so far). **No new dependencies.**

**Storage**: N/A — no new persisted state. Access-check results live only in the in-memory
`rbacAccess.selfAccess` store slice for the life of the session/context.

**Testing**: `cargo test` (unit tests for the new `CheckResourceMutationAccess` handler using a
fixture/mock discovery path where feasible, plus existing `resolve_namespace_access`/self-access
tests untouched), `tests/protocol_serde.rs` round-trip for the new command/event pair,
`pnpm -C ui test` (Vitest for `useMutationAccess` and the store-merge fix), one new Playwright e2e
spec (`ui/tests/e2e/us1-mutation-access-gating.spec.ts`) covering node cordon as the representative
surface (denied vs. allowed identity).

**Target Platform**: Same Tauri v2 desktop app, Tier 1 platforms per constitution (no platform-
specific behavior in this feature).

**Project Type**: Desktop app (Rust backend + React/TS frontend), existing structure.

**Performance Goals**: One `RefreshRbacSelfAccess`/`CheckResourceMutationAccess` round trip per
panel-open (not per render, not per keystroke) per Principle V; results are cached in the store
slice and reused across re-renders and by any other component asking the same question in the
same session.

**Constraints**: Per FR-005/Principle VI, this never changes server-side enforcement — every
check is advisory UI affordance only, and a check that cannot be determined (loading/error) MUST
NOT render a false "not permitted" tooltip (FR-006) — it falls back to the pre-feature enabled
state.

**Scale/Scope**: New Rust: one new command/event pair (`CheckResourceMutationAccess` /
`ResourceMutationAccessChecked`) plus its handler in `rbac_access.rs` reusing `apply.rs`'s
discovery helper; two exhaustive-match arms added in `mutation.rs` (compiler-enforced). New
frontend: one shared hook (`ui/src/hooks/useMutationAccess.ts`), one shared disabled-button/
tooltip helper, a store-merge fix in `store/handlers/rbacAccess.ts`, wiring into
`NodeActionsPanel.tsx`, `YamlEditTab.tsx`/`editSession.ts`, `HelmUpgradePanel.tsx`,
`resources/helm.tsx` (rollback), `HelmReleaseActionsPanel.tsx` (uninstall), and
`RoleBindingForm.tsx` (refactor) + `resources/rolebindings.tsx`/`resources/clusterrolebindings.tsx`
(new delete gating via one optional `ActionDescriptor` field).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **I. Operator-First, Explicit Mutation**: PASS. This feature does not add or change any
  confirmation step — it only changes whether the control that opens one is enabled. Every
  existing `ConfirmDialog` flow is unchanged for identities with access.
- **II. One Contract, Backend Owns the Cluster**: PASS. The one new capability
  (`CheckResourceMutationAccess`) is a new versioned command/event pair like every other; no new
  backend internals are exposed to the UI, and discovery/SSAR logic stays server-side. The
  frontend hook only orchestrates dispatch + cached lookup of already-typed protocol data.
- **III. Tested at the Boundaries**: PASS (planned). New protocol variants get a serde round-trip
  test; the new handler gets unit coverage; the shared hook and store-merge fix get Vitest
  coverage; one Playwright e2e spec covers the representative allow/deny flow end-to-end against
  a scripted fake backend, per existing `us2-node-actions.spec.ts`-style convention.
  `cargo clippy -- -D warnings` and the full frontend lint/typecheck/test/e2e suite must stay
  green.
- **IV. Density Scales With Width**: PASS. No new panes/panels; only existing buttons gain a
  `disabled`+tooltip state. Tooltips must still meet WCAG 2.2 AA contrast (reuses the existing
  themed `Tooltip` component, not a new one).
- **V. Fast and Quiet**: PASS. One batched check per panel-open, cached and reused; no new
  watches/polling. This is explicitly better than today's `RoleBindingForm.tsx` precedent, which
  already does exactly one check per mount/dependency-change — the new shared hook keeps that
  discipline and adds cross-component reuse via the store-merge fix instead of re-fetching.
- **VI. User Data Is Sacred**: PASS — directly reinforces "Fathom never escalates privileges": this
  feature's entire purpose is making the UI *more* honest about the operator's real permissions,
  never less.

No violations. Complexity Tracking table not needed.

## Project Structure

### Documentation (this feature)

```text
specs/018-rbac-aware-mutation-ui/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
src/
├── protocol/v1/
│   ├── cmd.rs                 # + CheckResourceMutationAccess command variant
│   ├── event.rs               # + ResourceMutationAccessChecked event variant
│   ├── mutation.rs            # + match arms for the new command (exhaustive, compiler-enforced)
│   └── rbac_access.rs         # (unchanged types; new event's payload reuses RbacAccessResult shape)
├── backend/
│   ├── apply.rs                # expose a small reusable discovery helper (resolve plural/group)
│   ├── rbac_access.rs          # + handle_check_resource_mutation_access (reuses SSAR + discovery)
│   └── cmd/resources.rs        # + dispatch arm for the new command

ui/
├── src/
│   ├── hooks/useMutationAccess.ts        # NEW: shared point-lookup hook
│   ├── store/handlers/rbacAccess.ts      # merge (not overwrite) selfAccess results
│   ├── store/index.ts                    # (if needed) selfAccess typing tweak for merge map
│   ├── components/panels/NodeActionsPanel.tsx      # wire cordon/uncordon/taint/drain
│   ├── components/panels/YamlEditTab.tsx           # wire apply (+ editSession.ts create/update verb)
│   ├── components/panels/HelmUpgradePanel.tsx      # wire upgrade
│   ├── resources/helm.tsx                          # wire rollback
│   ├── components/panels/HelmReleaseActionsPanel.tsx # wire uninstall
│   ├── components/rbac/RoleBindingForm.tsx         # refactor onto shared hook (no behavior change)
│   ├── resources/rolebindings.tsx                  # gate delete via ActionDescriptor.accessCheck
│   ├── resources/clusterrolebindings.tsx           # gate delete via ActionDescriptor.accessCheck
│   ├── resources/types.ts                          # + optional ActionDescriptor.accessCheck field
│   └── components/ActionMenu.tsx                   # consult accessCheck, render disabled+tooltip
└── tests/e2e/us1-mutation-access-gating.spec.ts     # NEW hyper-focused e2e (node cordon)
```

**Structure Decision**: Existing desktop-app structure (Rust backend + `ui/` React frontend)
reused as-is; no new top-level directories. This feature is additive within existing modules,
following spec 011's precedent closely (same files it touched: `rbac_access.rs`, `cmd.rs`,
`event.rs`, `mutation.rs`, `store/handlers/rbacAccess.ts`).

## Complexity Tracking

*No Constitution Check violations — table omitted.*

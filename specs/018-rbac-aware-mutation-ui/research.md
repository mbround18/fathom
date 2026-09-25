# Phase 0 Research: Proactive RBAC-Aware Mutation UI

## Decision 1: Which spec-011 mechanism to extend

**Decision**: Extend and generalize `RefreshRbacSelfAccess`/`RbacAccessCheck`/`RbacAccessResult`
(live `SelfSubjectAccessReview` point-lookup), not `resolve_namespace_access()`'s enumeration join.

**Rationale**: `resolve_namespace_access()` requires the caller to be able to **list**
`Role`/`ClusterRole`/`RoleBinding`/`ClusterRoleBinding` — a cluster-admin-level permission most
operators using node actions, Helm actions, or YAML apply will not have. `RefreshRbacSelfAccess`
performs a live `SelfSubjectAccessReview`, which every authenticated identity can call about
itself regardless of whether it can list RBAC objects — this is precisely why Kubernetes provides
`SelfSubjectAccessReview` as a distinct, universally-callable review kind. It is also already a
generic `(verb, resource, namespace) → allowed`/`reason` point-lookup, batched (`Vec<RbacAccessCheck>`
per command), and already gates one production surface (`RoleBindingForm.tsx`). The spec's own
wording — "SelfSubjectAccessReview-equivalent semantics" — describes this mechanism exactly; it
does not need to be reinvented on top of the enumerator.

**Alternatives considered**:
- *Extend `resolve_namespace_access()` with a point-lookup helper*: rejected as the *primary*
  mechanism for the reason above (list-permission requirement), though its existing join logic
  remains the right tool for the RBAC-admin-only "what can this identity do" browsing view it
  already powers, and is not being removed or duplicated.
- *Build a brand-new authorization/check abstraction*: rejected — would duplicate
  `RefreshRbacSelfAccess`, contradicting User Story 2's explicit "one shared mechanism" requirement
  and Principle II.

## Decision 2: Handling YAML apply/create's dynamically-discovered resource name

**Decision**: Add one new command, `CheckResourceMutationAccess { api_version, kind, namespace,
verb }`, whose handler reuses `apply.rs`'s existing `kube::discovery::oneshot::pinned_kind` call to
resolve the true plural/group, then runs the same `SelfSubjectAccessReview` logic
`handle_refresh_rbac_self_access` already uses.

**Rationale**: Every other in-scope surface (node actions, Helm actions, RBAC admin bindings) has
a statically-known Kubernetes resource name (`nodes`, `secrets`, `rolebindings`,
`clusterrolebindings`) that the frontend can pass directly to the existing generic
`RefreshRbacSelfAccess` command with zero backend changes. YAML apply/create is the one surface
where the resource is *whatever kind the operator is editing* — guessing its plural client-side
(e.g. naive `kind.toLowerCase() + 's'`) would be wrong for a meaningful fraction of built-in kinds
(`Ingress` → `ingresses`, `NetworkPolicy` → `networkpolicies`, and arbitrarily for CRDs), and
`apply.rs` already solves exactly this problem via cluster discovery for the apply/create call
itself. Reusing that discovery path for the access pre-check keeps "one contract, backend owns the
cluster" (Principle II) and guarantees the check and the eventual real mutation agree about what
resource is being acted on.

**Alternatives considered**:
- *Client-side pluralization heuristic table*: rejected — no such table exists today (confirmed by
  code search), would need ongoing maintenance for irregular plurals and every CRD, and could
  silently disagree with what `apply.rs` actually resolves.
- *Skip gating YAML apply/create entirely*: rejected — it is one of the four explicitly named
  entry points in the feature description.

## Decision 3: Mapping Helm's client-side actions onto Kubernetes verbs/resources (FR-008)

**Decision**: `upgrade`/`rollback` → `update` on `secrets` in the release's namespace; `uninstall`
→ `delete` on `secrets` in the release's namespace.

**Rationale**: Helm 3 stores release state as Kubernetes `Secret` objects (`sh.helm.release.v1.*`)
in the release's namespace. An identity that cannot write/delete Secrets there cannot actually
complete a Helm mutation against a real cluster (helm's own client library performs these writes
as part of every mutating operation), so this is a faithful, if approximate, stand-in for "can
this Helm action succeed" without needing to model Helm's internal RBAC needs exactly (explicitly
called out as an accepted approximation in spec.md's Assumptions).

**Alternatives considered**:
- *Skip Helm gating*: rejected — Helm upgrade/rollback/uninstall are explicitly named entry points.
- *Model exact per-chart RBAC (every object kind the chart would touch)*: rejected as far beyond
  "gate existing controls" scope; would require a full dry-run/diff of the chart's rendered
  manifests against RBAC, which is a permission-explorer-scale feature the spec explicitly rules
  out ("Don't build a full permission-explorer UI").

## Decision 4: Frontend caching/reuse strategy

**Decision**: A shared `useMutationAccess(checks)` hook dispatches any not-yet-known checks once
per mount/dependency-change (mirroring `RoleBindingForm.tsx`'s existing `useEffect` pattern
exactly) and reads results from a `rbacAccess.selfAccess` store slice that is fixed to **merge**
incoming `RbacAccessResult`s by `(verb, resource, namespace)` key instead of overwriting the whole
array on every `RbacSelfAccessUpdated` reply.

**Rationale**: Today, `store/handlers/rbacAccess.ts` replaces `selfAccess` wholesale on every
reply (`d.edit('rbacAccess').selfAccess = event.data.results`). This is harmless with exactly one
consumer (`RoleBindingForm.tsx`, which only ever has one check in flight), but breaks the moment a
second surface (e.g. a Node actions panel and a Helm panel open in different tabs/panes at once)
each dispatch checks — the second reply would wipe out the first's still-relevant result.
Switching to keyed-merge is the minimum change needed for "one shared mechanism, safely reusable
by every surface" (User Story 2) to actually be safe.

**Alternatives considered**:
- *Per-surface local state, no shared store change*: rejected — reintroduces exactly the
  duplication User Story 2 is written against, and does not fix the underlying overwrite bug for
  any future consumer.
- *Full request-response correlation via `RequestId`*: unnecessary — the existing check-tuple key
  is already a natural, stable identity for the answer to "can I do X", and every check is
  idempotent/side-effect-free, so last-write-wins **per key** (not per whole array) is sufficient.

## Decision 5: Hide vs. disable

**Decision**: Disable (kept visible, inactive, tooltip explains why) — already decided in spec.md
Assumptions; recorded here only to confirm it drives implementation (no hidden controls).

## Decision 6: Loading/error fallback (FR-006)

**Decision**: While a check's result is not yet in the store (in flight or never requested), the
shared hook reports `status: 'unknown'`; call sites treat `'unknown'` the same as "allowed" for
the purpose of enabling the control (today's pre-feature behavior — server-side/guard-check
enforcement remains the backstop), and never render a tooltip in that state. Once a definite
`allowed: false` result arrives, the control flips to disabled with its tooltip; once
`allowed: true` arrives (or never changes because it was already enabled), no visible change
occurs.

**Rationale**: Matches spec.md's Edge Cases and FR-006 exactly: never show a false "not permitted"
tooltip, and never introduce new friction for identities that already have access.

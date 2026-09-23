# Research: Pod Exec Session Gate

## R1: How does the existing mutation guard classify commands, and how should `StartPodExec` be reclassified?

**Decision**: Reclassify `BackendCmdV1::StartPodExec` from `MutationTarget::ReadOnly` to
`MutationTarget::Namespaced { namespaces: vec![namespace] }` in
`src/protocol/v1/mutation.rs`. `SendPodExecInput`, `ResizePodExec`, and `StopPodExec` stay
`ReadOnly` (they act on an already-gated, already-open session; only the *opening* act — the
one with cluster-reaching side effects — is gated, matching the spec's framing of "opening an
exec session" as the gated action).

**Rationale**: `mutation_target()` already drives a real enforcement point:
`tools/devbridge/src/guard.rs`'s sandbox guard (used when Fathom's own backend is driven over
the dev bridge, e.g. by an agent) allows `ReadOnly`/`Local` commands unconditionally but
requires an explicit namespace allow-list for `Namespaced`/`ClusterScoped` writes
(`MutationTarget::writes_cluster()` — see `guard.rs` lines 37-61). Reclassifying `StartPodExec`
this way means an exec session cannot be opened through that guard against a namespace nobody
allow-listed — exactly the "gate exec like a mutation" requirement — with zero new guard code.
It also changes `finished_route()` in `src/backend.rs` to route `StartPodExec`'s
`CommandFinished` event to the target namespace, consistent with every other single-namespace
write.

**Alternatives considered**:
- A brand new `MutationTarget::Sensitive` variant: rejected — it would need its own guard
  arm in `tools/devbridge/src/guard.rs` and its own `writes_cluster()` semantics, for no
  behavioral difference from reusing `Namespaced` (exec never touches more than one
  namespace).
- Leaving `mutation_target()` untouched and gating only in the UI: rejected — it satisfies
  FR-002/FR-003 (the confirmation) but not FR-001 ("use the same guard mechanism"), and misses
  the free defense-in-depth on the dev-bridge path.

## R2: What live permission-check primitive already exists, and can it be reused as-is?

**Decision**: Add a new, narrowly-scoped backend command, `CheckPodExecAccess { namespace,
name, container }`, that issues one `SelfSubjectAccessReview` with
`resource: "pods"`, `subresource: "exec"`, `verb: "create"`, `group: ""` (core API group),
`namespace`, and `name`. This lives in `src/backend/exec.rs` next to `start_exec`, reusing the
`Api<SelfSubjectAccessReview>` + `PostParams::default()` call shape already established in
`src/backend/rbac_access.rs::handle_refresh_rbac_self_access`.

**Rationale**: The feature brief pointed at `resolve_namespace_access()` (spec 011,
`src/backend/rbac_access.rs`), but that function resolves what an *arbitrary subject* (by
name/kind, for RBAC administration screens) can access by statically joining
Role/ClusterRole/RoleBinding/ClusterRoleBinding objects — it answers "what can subject X do"
for an admin browsing bindings, not "can the operator who is currently driving this app do
this right now." The live check this spec actually needs is a `SelfSubjectAccessReview`
against the API server, which is exactly what `handle_refresh_rbac_self_access` already does
for spec 011's own self-access panel — except that handler hardcodes
`group: "rbac.authorization.k8s.io"` (`src/backend/rbac_access.rs`, the `ResourceAttributes`
literal), because every check it currently serves is itself an RBAC-object check
(`roles`, `rolebindings`, etc.). Widening `RbacAccessCheck`'s hardcoded group would be a
cross-cutting protocol change to an existing, already-shipped command for a single new
call site — a new dedicated command is smaller and keeps `RefreshRbacSelfAccess`'s existing
contract untouched.

**Alternatives considered**:
- Generalize `RbacAccessCheck`/`RefreshRbacSelfAccess` to take an explicit `group`/
  `subresource`: viable, slightly more reusable, but touches a shipped spec-011 contract and
  its existing tests/UI consumers for one new caller. Deferred; noted as a possible follow-up
  cleanup, not required for this spec's acceptance criteria.
- Client-side RBAC inference (reuse `resolve_namespace_access()` against locally-cached
  Role/RoleBinding objects for the current operator's own identity): rejected — the backend
  does not reliably know the current operator's own subject identity/groups (that's an
  API-server-side concept tied to the credential in use), so a live `SelfSubjectAccessReview`
  is the only mechanism that answers "can *I* do this" without guessing the caller's identity.

## R3: Does this spec need any new Rust or JS dependency?

**Decision**: No new dependency. `kube`'s `ws` feature (already enabled in `Cargo.toml` for
port-forwarding and exec) covers the SPDY/WebSocket exec channel; `k8s-openapi`'s
`authorization::v1` types (already a transitive dependency, already used by
`rbac_access.rs`) cover `SelfSubjectAccessReview`/`ResourceAttributes`; `xterm` +
`@xterm/addon-fit` (already in `ui/package.json`, used by `PodTerminalTab.tsx`) render the
terminal; the existing `ConfirmDialog` component (`ui/src/components/ConfirmDialog.tsx`)
covers the confirmation UI. This spec only adds one new protocol command/event pair and
~1 new UI phase to an existing component tree.

**Rationale**: Checked `Cargo.toml` and `ui/package.json` first per project discipline; both
already carry everything required.

## R4: What does the UI confirmation reuse, and what changes in `PodTerminalTab.tsx`'s flow?

**Decision**: Reuse `ConfirmDialog` (`ui/src/components/ConfirmDialog.tsx`) directly inside
`PodTerminalTab.tsx`, not through `ActionDialogHost`/the row-action descriptor system
(`ui/src/components/ActionDialogHost.tsx`), because the exec confirmation isn't a list-row
action with a `plan()` — it belongs to a single already-selected pod inside an already-open
Inspector tab. `PodTerminalTab` gets one item (`{ namespace, name }`, with the chosen
container named in `detail`) and calls `ConfirmDialog` with `destructive: true`,
`confirmLabel: "Open terminal"`.

The existing auto-attach behavior (today: choosing/defaulting a container immediately sends
`StartPodExec`) is replaced with an explicit multi-step flow:
`picking` (container, if >1) → `checking` (send `CheckPodExecAccess`, await result) →
`denied` (permission check said no; terminal state until the operator picks a different
pod/container) → `confirming` (send nothing yet; `ConfirmDialog` shown) → `attaching` (send
`StartPodExec`, exactly as today) → `active`/`ended`/`error` (unchanged).

**Rationale**: FR-002 requires the confirmation before any attach request; FR-004 requires the
permission pre-check to run, and pre-empt the confirmation, before that. Reusing the existing
phase state machine (already tracked via a `Phase` union in the component) keeps the change
additive rather than a rewrite.

**Alternatives considered**: Firing the permission check as soon as the Inspector opens
*any* tab for the pod (before the operator switches to Terminal) was considered, but rejected
— it would run a `SelfSubjectAccessReview` for pods the operator never intends to exec into.
Instead, the check fires at the same trigger point today's code already uses to auto-attach
(container resolved, whether by picker choice or the single-container default) — i.e. as soon
as the operator is on the Terminal tab with a container chosen — which matches today's
existing latency/affordance and adds no extra step beyond the confirmation itself.

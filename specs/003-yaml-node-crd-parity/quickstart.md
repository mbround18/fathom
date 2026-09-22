# Quickstart: Validating Resource Editing, Node Actions & CRD Browsing

## Prerequisites

- A running kubeconfig context with cluster-admin-equivalent RBAC (to exercise all three stories),
  reachable the same way any other Fathom feature is validated (`FATHOM_LIVE_CONTEXT=<context>`
  for live-cluster E2E, or the scripted fake-backend fixture for deterministic Playwright runs —
  see `002-pod-logs-terminal/quickstart.md` for the fixture-driving convention this reuses).
- At least one installed CRD with an existing instance (e.g. Flux's `GitRepository`, already
  present on any Flux-enabled test cluster) to validate US3 without needing to install a new one.
- A node that is safe to cordon/drain/uncordon in the test cluster (a worker node with no bare
  pods, ideally a multi-node cluster so draining doesn't take the whole cluster offline).

## User Story 1 — Edit YAML + apply

1. Open any Deployment in the Inspector, switch to the YAML tab.
2. Edit a non-identity field (e.g. bump a `replicas` count or add a label) in the edit buffer.
3. Click "Review changes" — confirm the diff view shows exactly the lines changed, nothing else.
4. Confirm apply — confirm the dialog names the exact object being changed (Principle I), then
   confirm the object updates and the tab reflects the new live state.
5. Repeat, this time editing `metadata.name` — confirm apply is rejected client-side or
   server-side before any cluster write, with a clear "identity fields cannot change" error
   (FR-005).
6. Repeat again: open the YAML tab, note the `resourceVersion`, apply a change from `kubectl`
   directly against the same object, then attempt to apply the still-open (now-stale) edit —
   confirm apply fails with a specific version-conflict error and the edit buffer is preserved,
   not silently discarded (FR-004/FR-007).
7. Edit the YAML, then navigate away/close the tab without applying — confirm a discard warning
   appears (FR-006).

**Expected**: SC-001 (<60s edit-to-applied-confirmed), SC-002 (100% rejection of identity-changing
or stale-version applies pre-mutation).

## User Story 2 — Node actions

1. Select a worker node, cordon it — confirm the dialog names the node, confirm, and verify
   `kubectl get node` shows `SchedulingDisabled`.
2. Uncordon it — confirm the round trip completes and scheduling is re-enabled.
3. Drain a node with a small number of evictable pods (and at least one DaemonSet-owned pod) with
   "ignore DaemonSets" checked — confirm live progress ("N of M pods evicted"), confirm the
   DaemonSet pod is left alone and reported as ignored, confirm final state is "complete".
4. If a PDB-protected pod is present, drain a node hosting it without any bypass — confirm the pod
   shows as "blocked" in the progress view with the server's own reason surfaced, and the drain
   does not silently skip past it as if evicted.
5. Attempt to drain a node hosting a bare pod (no controller owner) — confirm the drain refuses
   up front with a clear message, matching `kubectl drain`'s default (non-force) behavior.
6. Start a drain, cancel it mid-way — confirm it stops cleanly and the progress view reports which
   pods were evicted before cancellation.
7. Add a taint, confirm it appears in `kubectl describe node`; remove it, confirm it's gone. Both
   gated behind the same confirmation dialog.

**Expected**: SC-003 (<30s cordon-confirm-uncordon round trip), SC-004 (100% eviction completion
for an unblocked drain with visible per-step progress), SC-005 (PDB blocks identified without
external tooling).

## User Story 3 — Browse Custom Resources

1. Open the new "Custom Resources" nav section — confirm it lists every installed CRD (not just
   Flux/Longhorn), grouped sensibly (e.g. by API group).
2. Select a CRD kind with existing instances (e.g. Flux's `GitRepository`) — confirm instances
   list, correctly scoped (namespaced kinds show a namespace column/filter; cluster-scoped kinds
   don't).
3. Open an instance — confirm Details/YAML/Events tabs render the same way they do for a built-in
   kind.
4. On a cluster/context with zero CRDs installed, confirm the section shows an explicit empty
   state, not an error.
5. If RBAC denies listing one particular CRD's instances (e.g. via a restricted test
   ServiceAccount/context), confirm that CRD alone shows a permission-denied state while every
   other CRD in the catalog still lists normally (FR-019).

**Expected**: SC-006 (<45s CRD-discovery-to-YAML-inspection for an unfamiliar CRD).

## Cross-cutting verification

- `cargo test` — new protocol round-trip tests for all ten new command/event variants pass.
- `cargo clippy --all-targets -- -D warnings` — exhaustive-match compile guarantees hold (a
  forgotten `mutation_target()`/`section()` arm fails the build, not just a lint).
- `pnpm -C ui typecheck && pnpm -C ui lint` clean.
- Playwright E2E (`ui/tests/e2e/`) scenarios for apply+diff+conflict, cordon/drain/taint, and
  CRD browse/empty/permission-denied all pass against the scripted fake backend.
- Manually confirm zero mutating action in any of the above steps ever executes without an
  explicit confirmation click (SC-007) — this should be self-evident from the UI, not something
  requiring a special test harness.

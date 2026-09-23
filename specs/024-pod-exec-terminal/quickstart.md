# Quickstart: Validating the Pod Exec Session Gate

Prerequisites: a running dev cluster with a `Pod` fixture (e.g. `dev/sandbox/workloads.yaml`'s
`web` Deployment pod), Fathom's Tauri dev shell running against it (`pnpm -C ui dev` +
`cargo run`, or the existing `make`/`Makefile.toml` dev target), and two kubeconfig identities:
one with `pods/exec` `create` in the pod's namespace, one without.

## Scenario 1 — Happy path (User Story 1)

1. Select the fixture pod in the Inspector, open its **Terminal** tab.
2. If the pod has more than one container, pick one.
3. Observe the tab shows a brief "checking permissions" state, then a confirmation dialog
   naming the exact namespace/pod/container (`ConfirmDialog`, `data-testid="confirm-dialog"`).
4. Click confirm. Expect: the existing attach/streaming flow proceeds unchanged — the shell
   attaches, and typed input (e.g. `echo hello`) round-trips through the rendered `xterm.js`
   pane (`data-testid="pod-terminal-xterm"`).
5. Reopen the Terminal tab (switch away and back, or reselect the pod). Expect: the
   confirmation is shown again — it is never remembered.
6. Repeat step 1-3, then click Cancel/Esc. Expect: no `StartPodExec` command is sent (verify
   via the fake-backend E2E harness's command log) and the pane returns to its
   pre-confirmation state.

## Scenario 2 — RBAC gate (User Story 2)

1. Switch to the kubeconfig identity without `pods/exec` permission in the fixture pod's
   namespace (or, for the E2E fake-backend test, inject a `PodExecAccessChecked` event with
   `allowed: false`).
2. Select the fixture pod, open its Terminal tab (pick a container if prompted).
3. Expect: a permission-denied message appears immediately; no `ConfirmDialog` is ever shown;
   no `StartPodExec` is sent.
4. Switch back to the permitted identity and repeat Scenario 1 to confirm the gate is specific
   to the permission result, not a general regression.

## Automated coverage

- `src/backend/exec.rs` unit tests: `CheckPodExecAccess` review construction (correct
  `resource`/`subresource`/`verb`/`group`/`namespace`/`name`) and its error-mapping to
  `allowed: false` + verbatim reason.
- `src/protocol/v1/cmd.rs`/`event.rs`: serde round-trip tests for `CheckPodExecAccess` and
  `PodExecAccessChecked` (Constitution Principle III).
- `src/protocol/v1/mutation.rs`: a test asserting `StartPodExec`'s `mutation_target()` is
  `Namespaced` (not `ReadOnly`), alongside the existing exhaustive-match compile-time
  guarantee.
- `ui/tests/e2e/pod-exec-gate.spec.ts`: the two scenarios above, driven against the fake
  backend, per FR-002/FR-003 (confirmation) and FR-004/FR-005 (RBAC pre-check).

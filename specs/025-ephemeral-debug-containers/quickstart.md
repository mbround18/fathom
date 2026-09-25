# Quickstart: Validating Ephemeral Debug Containers

## Prerequisites

- A Kubernetes cluster (kind/minikube is fine) at 1.25+ with a running pod, e.g.:
  ```sh
  kubectl run demo --image=nginx --restart=Never
  ```
- Fathom built and pointed at that cluster's kubeconfig context.
- RBAC permission to `patch` the `pods/ephemeralcontainers` subresource in the target namespace
  (grant `verbs: ["patch","update"]` on `resources: ["pods/ephemeralcontainers"]` if testing
  against a restricted service account).

## Manual validation (against a real cluster)

1. Open the pod detail view for `demo`.
2. Trigger "Add debug container." Enter image `busybox` (or `nicolaka/netshoot`); with only one
   container present, the target container is auto-selected.
3. Confirm the dialog shows: pod name `demo`, target container `demo`, image `busybox`, and the
   "cannot be removed" warning. Confirm.
4. Watch the pod's container list: a new entry appears (name auto-generated, e.g.
   `debugger-xxxxx`) with status transitioning waiting → running.
5. Select the new container in the container list and open its logs — confirm the existing log
   viewer streams output from it with no special-casing needed.
6. If the pod-exec terminal is available in this build, launch exec against the new container the
   same way as any other container and confirm an interactive shell opens.
7. Confirm there is no "remove" affordance anywhere for this container (FR-009) — it persists for
   the pod's lifetime, matching the warning shown in step 3.

Expected: see `spec.md` Success Criteria SC-001–SC-004.

## Automated validation (fake-backend e2e)

Location: `ui/tests/e2e/ephemeral-debug-container.spec.ts` (Playwright, against the project's
existing scripted fake backend harness — see other specs' `ui/tests/e2e/*.spec.ts` for the
harness pattern).

Single focused scenario (per implementation brief — kept tight, one end-to-end path):
1. Load a fixture pod with a single container.
2. Open "Add debug container," fill in an image name, submit.
3. Assert the confirm dialog is visible and its text contains the "cannot be removed" warning.
4. Confirm the dialog; assert the fake backend received `AddEphemeralContainer` with the expected
   fields.
5. Script the fake backend to report the mutation's `CommandFinished` success and an updated pod
   detail/`containers_lifecycle` including the new ephemeral entry.
6. Assert the new container appears in the pod's container list in the UI.

## Contract/unit validation

- `cargo test` — protocol serialization round-trip for `AddEphemeralContainer`
  (`src/protocol/v1/cmd.rs` test module, following the existing `roundtrip(...)` pattern seen for
  `StartPodExec`), plus a `mutation.rs` unit test asserting
  `AddEphemeralContainer { .. }.mutation_target()` returns
  `MutationTarget::Namespaced { .. }` with `writes_cluster() == true`.

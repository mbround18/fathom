# Live validation: 030-kind-cluster-provisioning (T050)

Run 2026-09-26 on Linux (Pop!_OS 24.04, Docker 29.8.1, kind v0.27.0 with its default node image kindest/node:v1.32.2).

**Harness.** The real backend ran headless via `examples/live_driver.rs`, with isolated `HOME` and `KUBECONFIG`. The cluster names used were `fathom-prov-manual` (created from a terminal) and `fathom-prov-demo` (created by the app); both were deleted afterwards. No WebDriver is available here, so dialogs and panel rendering are covered by `ui/tests/e2e/kind-cluster-provisioning.spec.ts` (fake backend), not this run.

| Quickstart step | Result |
|-----------------|--------|
| 1. Prerequisite detection | With `kind` removed from PATH: `kind_installed: false, runtime: docker (running), ready: false`. A create sent anyway was refused by the backend re-check with `kind is not installed.` After restoring PATH: `kind v0.27.0 go1.23.6 linux/amd64`, `ready: true`. **Pass.** |
| 2. Pre-existing cluster | `kind create cluster --name fathom-prov-manual` (terminal, 24.8s) is listed as `ready` with context `kind-fathom-prov-manual`. The real `kind get clusters` output parsed correctly alongside 10 other clusters on the host. **Pass.** |
| 3. Create in app | `fathom-prov-demo` with the `bare` profile succeeded in 17.8s (SC-001: under 2 min). The real kind progress lines were streamed as stages: `Ensuring node image`, `Preparing nodes` (+5.8s), `Writing configuration`, `Starting control-plane` (+15.5s), `Installing CNI`, `Installing StorageClass`, `Set kubectl context`. `ContextsUpdated` contained `kind-fathom-prov-demo` in the same batch as the finish event, and `SetContext` to it succeeded (SC-005). **Pass.** |
| 4. Delete in app | Deleting the active context took 1.0s (`Deleted nodes: [...]`). The context left `ContextsUpdated`, `current` became `None`, and the cluster left the list. **Pass.** |
| 5. Invalid or colliding names | `Manual_Check!` was rejected: `Cluster name may only contain lowercase letters, digits, and '-'.` `fathom-prov-manual` was rejected: `A kind cluster named 'fathom-prov-manual' already exists.` Neither produced any progress event, so no process was spawned (SC-006). **Pass.** |

**Finding (fixed).** Real kind prints nothing for about 9.4s while the control plane starts: this was the largest gap between progress lines. The panel previously showed a static stage label for that whole time, which is borderline for SC-004 ("never appears frozen"). `KindClusterPanel` now shows a ticking elapsed counter (`(12s)`) while an operation is in progress. The Playwright spec asserts that the counter advances.

**Not exercised live:**
- the confirmation dialogs (steps 3 and 4 UI), which are covered by the fake-backend e2e;
- Podman;
- the `custom_yaml` profile.

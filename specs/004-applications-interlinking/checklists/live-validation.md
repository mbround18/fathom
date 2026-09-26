# Live validation: 004 Applications interlinking, Scenario 4

- **Date**: 2026-09-26
- **Cluster**: `kind-fathom-dev`, namespace `fathom-dev`, with Flux installed from `k8s/addons/flux`
- **How**: `ui/tests/live/qs-004-applications-edge.spec.ts` through the devbridge (`chromium-wide`)

| Step | Result | Notes |
|---|---|---|
| 4.1-4.2 a hand-applied Deployment with no `app.kubernetes.io/*`, Helm or Flux metadata is absent from Applications but still listed under Deployments | PASS | |
| 4.3-4.4 an application with zero remaining workloads stays listed with "No workloads" (FR-016) | PASS (after fix) | The Flux `podinfo` Deployment was deleted out of band. It had been dropped from the list; it now stays listed. The spec's forced reconcile restores it. |

The Flux attribution in the test (the Inspector shows "Managed by Flux") exercises the label-based
attribution recorded as the T016 deviation.

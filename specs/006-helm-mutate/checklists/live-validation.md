# Live validation: 006 Helm mutate

- **Date**: 2026-09-26
- **Cluster**: `kind-fathom-dev`, namespace `fathom-dev`
- **Releases**:
  - `podinfo`: Flux-managed, from `dev/sandbox`.
  - `quickstart-bare`: a plain `helm install` of `oci://ghcr.io/stefanprodan/charts/podinfo`
    6.15.0, with rev 2 = `--set ui.message=qs006-before`. It is re-installed after each run,
    because S3 uninstalls it.
- **How**: `ui/tests/live/qs-006-helm-mutate.spec.ts` through the devbridge (`chromium-wide`)

| Scenario | Result | Notes |
|---|---|---|
| S1 rollback: the preview changes nothing; cancelling at the preview or at the confirm adds no revision; confirming adds "Rollback to 1" | PASS (after fix) | also asserts exactly one `deployed` revision and that the live pod template changed |
| S1.6 rollback of the Flux release warns and names the HelmRelease | PASS | cancelled |
| S2 upgrade: effective values preload, invalid YAML errors at preview, the dry-run diff shows `replicas: 2`, applying adds a revision and scales | PASS | |
| S2.5 upgrade of the Flux release warns before the diff | PASS | never applied |
| S3 uninstall: the Flux release warns (cancelled); the bare release can be cancelled, then confirmed, and its resources are gone | PASS | |
| helm binary absent: degraded path | NOT RUN LIVE | covered by unit tests only (`helm_binary_missing` messages) |

## Bugs found and fixed

1. **Wrong cluster.** `helm upgrade`, `helm uninstall` and the upgrade preview ran without
   `--kube-context`, so they hit the kubeconfig's *current* context rather than the one selected
   in Fathom. Another tool switching contexts could have sent a mutation to production. Every helm
   shell-out now carries `--kube-context` (plus `--kubeconfig` when set), and refuses to run when no
   context is selected.
2. **Rollback never reached the cluster.** Rollback wrote a new release-storage Secret with the
   old payload. Nothing was applied, and the previous revision was left `deployed`, so two
   revisions showed as deployed. Rollback now runs `helm rollback`.

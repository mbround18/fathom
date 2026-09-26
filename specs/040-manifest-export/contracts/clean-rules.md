# Clean-for-re-apply rules (normative)

Implemented in `ui/src/features/manifest-export/clean.ts` (`cleanManifest`). Each rule has a Vitest case.

| # | Applies to | Rule |
|---|---|---|
| C1 | all | delete top-level `status` |
| C2 | all | delete `metadata.managedFields`, `resourceVersion`, `uid`, `creationTimestamp`, `generation`, `selfLink`, `deletionTimestamp`, `deletionGracePeriodSeconds` |
| C3 | all | delete `metadata.ownerReferences`. If any ref had `controller: true`, add a warning naming `<kind> <name>` |
| C4 | all | delete annotations `kubectl.kubernetes.io/last-applied-configuration`, `deployment.kubernetes.io/revision`, `pv.kubernetes.io/bind-completed`, `pv.kubernetes.io/bound-by-controller`, `pv.kubernetes.io/provisioned-by`, `volume.beta.kubernetes.io/storage-provisioner`, `volume.kubernetes.io/storage-provisioner`, and delete `annotations` when empty |
| C5 | Service | delete `spec.clusterIP` and `spec.clusterIPs` unless `spec.clusterIP === "None"` |
| C6 | Job | when `spec.manualSelector !== true`: delete `spec.selector`, and from `spec.template.metadata.labels` delete `controller-uid`, `batch.kubernetes.io/controller-uid`, `job-name`, `batch.kubernetes.io/job-name` |
| C7 | all | emit keys in the order `apiVersion`, `kind`, `metadata`, then the rest in original order |
| C8 | all | never mutate the input object |

Raw mode applies only C8.

Secret rules (redaction, and last-applied always dropped) are applied in the backend before the protocol (FR-011). The UI adds a "values redacted" warning for `kind: Secret`.

Rendering: `yaml` `stringify(obj, { lineWidth: 0 })`. Multi-document output is joined with a `---` line and terminated by `\n`.

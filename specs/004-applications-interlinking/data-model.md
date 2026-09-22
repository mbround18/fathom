# Data Model: Applications View & Rich Interlinking

## `ApplicationManagedBy`

Fieldless enum (serializes as a plain string), mirroring `CrdScope`'s convention:

- `Helm` — attributed via `meta.helm.sh/release-name`/`meta.helm.sh/release-namespace`
  annotations (R1), with no corresponding Flux object.
- `Flux` — attributed via a Flux `Kustomization`'s `status.inventory` (R2) or a Flux
  `HelmRelease`'s resolved Helm release name/namespace (R2, R3).
- `LabelHeuristic` — attributed via `app.kubernetes.io/name`/`app.kubernetes.io/part-of` labels
  alone, with neither Helm annotations nor Flux ownership found. This is the "label-only
  heuristic" tier named in spec.md FR-004; it exists so a resource with clear app-identity labels
  but no Helm/Flux install path (e.g. hand-applied manifests carrying standard k8s.io recommended
  labels) can still be grouped, but is visually distinguishable from a real Helm/Flux install.

## `ApplicationSummary`

One row in the Applications list (spec.md FR-004). Analogous in shape to other `*Summary` types
(e.g. `NodeSummary`).

| Field | Type | Notes |
|---|---|---|
| `instance` | `String` | Stable identity: Helm release name, Flux object name, or the label-heuristic grouping key. |
| `name` | `String` | Display name (same as `instance` for Helm/Flux; the `app.kubernetes.io/name` label value for the heuristic tier). |
| `namespace` | `String` | The release/Kustomization/HelmRelease's own namespace (cluster-scoped applications are out of scope for v1 — Helm releases and Flux objects are always namespaced). |
| `managed_by` | `ApplicationManagedBy` | See above. |
| `version` | `Option<String>` | Chart version (from `meta.helm.sh/release-name`'d resources' own chart annotation, when present) for Helm; Flux's `status.lastAppliedRevision` for a Kustomization; `None` for the label-heuristic tier. |
| `creation_timestamp` | `String` | Earliest `creationTimestamp` across the application's owned resources, matching how age is computed for every other list in Fathom. |
| `ready_count` | `i32` | Count of owned workloads (Deployments/StatefulSets/DaemonSets/etc., not raw pods) currently reporting ready. |
| `total_count` | `i32` | Total count of owned workloads. Ties to spec.md's aggregated-status requirement (FR-004) and the "no active workloads" edge case (FR-016, `total_count == 0`). |

## `ApplicationDetails`

Returned by selecting an application (spec.md FR-006–FR-008). Follows the same
`raw_json`-free, typed-fields convention as `WorkloadDetails`/`NodeDetails` (this is a synthesized
entity, not a single live Kubernetes object, so there is no single YAML/raw JSON to show — the
Inspector's YAML tab is not applicable to Applications, same as it isn't for the dashboard
Overview).

| Field | Type | Notes |
|---|---|---|
| `instance` | `String` | Same as `ApplicationSummary.instance`. |
| `name` | `String` | |
| `namespace` | `String` | |
| `managed_by` | `ApplicationManagedBy` | |
| `version` | `Option<String>` | |
| `creation_timestamp` | `String` | |
| `labels` | `BTreeMap<String, String>` | Union of labels Fathom used to identify the application (Helm annotations / Flux name / heuristic labels), for operator transparency. |
| `urls` | `Vec<String>` | Derived from owned `Service`s (`ClusterIP`/`LoadBalancer` with ports) and `Ingress` host rules; best-effort, matching `WorkloadDetails.pod_selector`'s existing "best-effort, empty if undeterminable" convention. |
| `related` | `Vec<RelatedResourceRef>` | Pods, Deployments/StatefulSets, ServiceAccounts, and owned CRD instances (spec.md FR-008), produced by the same generic resolver (R5) every other kind's inspector now uses. |

CPU/memory metrics are **not** a field on `ApplicationDetails` — they follow the existing
`WorkloadMetricsAggregate` pattern (already used for `DeploymentMetricsUpdated` etc.) as a
separate `ApplicationMetricsUpdated(WorkloadMetricsAggregate)` read event, keeping the "metrics
unavailable" state (FR-007) orthogonal to whether the application's identity data loaded.

## `RelatedResourceRef`

The generic relationship-resolver's (R5) output type — shared by every kind's inspector, not just
Applications. This is the protocol-level shape behind spec.md's Key Entity "Related Resource
Link."

| Field | Type | Notes |
|---|---|---|
| `section` | `SectionKey` | Which nav section/list this resource belongs to — lets the frontend build a correct `ObjectRef` without a second lookup. |
| `kind` | `String` | e.g. `"Pod"`, `"Deployment"`, `"Application"`. |
| `namespace` | `Option<String>` | `None` for cluster-scoped kinds (Node, ClusterRole, ...) or an Application in a future cluster-scoped tier (not applicable today — see `ApplicationSummary.namespace`, always `Some` for v1). |
| `name` | `String` | |
| `relation` | `String` | Short machine-readable label for grouping in the UI, e.g. `"owns"`, `"selects"`, `"scheduled-on"`, `"mounts"`, `"member-of"`. Free-form but drawn from a small fixed vocabulary documented in contracts/protocol.md, not user-facing prose. |
| `status_hint` | `Option<String>` | Optional cheap status summary (e.g. a Pod's phase) so the related-resources list can render a status badge without a second round trip per entry, mirroring how `WorkloadDetails.pods` already carries `phase`/`ready`/`total` inline today. |

`RelatedResourceRef` intentionally does **not** carry a resolved `ObjectRef` type directly (that's
a frontend-only concept in `ui/src/resources/types.ts`) — the frontend constructs an `ObjectRef`
from `{ section, kind, namespace, name }` the same way it already does for every list row today
(`ResourceDescriptor.ref(row)`), keeping the protocol layer UI-framework-agnostic per Constitution
Principle II.

## Relationship vocabulary (kind → relation → related kinds)

Concrete instantiations of `RelatedResourceRef.relation`, covering spec.md's Acceptance Scenario 3
and FR-010's minimum kind list:

| From kind | Relation | To kind(s) | Source |
|---|---|---|---|
| Application | `owns` | Deployment, StatefulSet, ServiceAccount, CRD instance | Helm/Flux attribution (R1–R3) |
| Application | `owns` | Pod | Transitive through owned Deployments/StatefulSets' pods |
| Deployment / StatefulSet / DaemonSet / ReplicaSet | `owns` | Pod | Already-fetched `pod_selector` (existing `WorkloadDetails` field) — no new lookup |
| Pod | `owned-by` | ReplicaSet/Deployment/StatefulSet/DaemonSet/Job | Pod's own `ownerReferences` (already present on the fetched object) |
| Pod | `member-of` | Application | Reverse of Application `owns`, resolved via the same Helm/Flux attribution applied to the pod's own metadata |
| Pod | `scheduled-on` | Node | Pod's own `spec.nodeName` (already present on the fetched object) |
| Node | `runs` | Pod | Targeted lookup: `Api<Pod>::list` filtered by `spec.nodeName == <node>` field selector |
| Service | `selects` | Pod | Targeted lookup: `Api<Pod>::list` with the Service's `spec.selector` as a label selector |
| PersistentVolumeClaim | `mounted-by` | Pod | Targeted lookup: `Api<Pod>::list` filtered to pods whose `spec.volumes[].persistentVolumeClaim.claimName` matches (bounded to the PVC's own namespace) |

## Frontend: drill-down / back-stack state (no protocol change — UI-only)

Extends `PaneState` (`ui/src/store/index.ts`), an Inspector pane only:

| Field | Type | Notes |
|---|---|---|
| `history` | `ObjectRef[]` | Existing `target`s the operator drilled through to reach the current `target`, oldest first (R6). Empty when the inspector was opened directly from a list row. |

No new top-level store slice, no new persisted `~/.fathom/` state (Constitution Principle VI is
unaffected — `history` is transient, cleared on pane close/re-open from a list row, never
persisted).

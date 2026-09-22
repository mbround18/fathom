# Data Model: Resource Editing, Node Actions & Custom Resource Browsing

## Resource Edit Session (frontend-held, ephemeral)

Tracks an in-progress YAML edit for the Inspector's YAML tab. Not persisted; lives only in
`StoreData` for the duration the Inspector is open on a given object.

| Field | Type | Notes |
|---|---|---|
| `target` | `{ apiVersion, kind, namespace?, name }` | The identity the session was opened for; identity fields are immutable for the life of the session (spec FR-005) |
| `openedResourceVersion` | `string` | `resourceVersion` captured when the YAML tab was opened; sent back on apply |
| `liveYaml` | `string` | Rendered via the existing `jsonToYaml()`; re-derived on every `raw_json` update the resource-watch stream delivers for the selected object while the session is open (live diff, not a static snapshot) |
| `editedYaml` | `string` | The operator's edit buffer; diverges from `liveYaml` as they type |
| `dirty` | `boolean` | `editedYaml !== liveYaml`; drives the discard-warning (spec FR-006) |
| `liveYamlChangedSinceReview` | `boolean` | Set when `liveYaml` updates after the operator last opened the diff view; drives a "content changed, re-review" indicator so a live diff shift is never silent |
| `status` | `'editing' \| 'diffing' \| 'applying' \| 'conflict' \| 'error' \| 'applied'` | Drives which UI affordance shows |
| `lastError` | `string?` | Verbatim backend error message on `'error'`/`'conflict'` (constitution Principle I: errors reach the operator verbatim) |

**Validation rules**:
- Apply is disabled unless `dirty` is true and `editedYaml` parses as YAML client-side (basic
  syntax check only — full structural validation happens backend-side).
- On `CommandFinished` failure classified as a 409 conflict, `status` becomes `'conflict'` and
  `editedYaml` is preserved unchanged (FR-007); the operator must explicitly re-open to refresh
  `liveYaml`/`openedResourceVersion` before retrying.

## Node Action (frontend-held, ephemeral; drain has a backend-held counterpart)

Represents the state of a confirmed node mutation in progress.

| Field | Type | Notes |
|---|---|---|
| `nodeName` | `string` | Target node |
| `kind` | `'cordon' \| 'uncordon' \| 'drain' \| 'taint-add' \| 'taint-remove'` | |
| `status` | `'confirming' \| 'in-progress' \| 'complete' \| 'cancelled' \| 'failed'` | |
| `taint` | `{ key, value?, effect }?` | Present only for `taint-add`/`taint-remove` |
| `drain` | `DrainProgress?` | Present only for `kind === 'drain'` |

**DrainProgress** (streamed incrementally from the backend, one event per pod outcome):

| Field | Type | Notes |
|---|---|---|
| `totalPods` | `number` | Snapshot count at drain start (excludes already-terminating pods) |
| `evicted` | `string[]` | Pod names successfully evicted so far |
| `blockedOn` | `{ pod: string; reason: string }[]` | Pods blocked by a PDB (429) or other eviction failure, with the server's own message surfaced verbatim |
| `ignoredDaemonSetPods` | `string[]` | Present if the operator chose "ignore DaemonSets" (FR-011) |
| `cancelled` | `boolean` | True once a `CancelDrainNode` has been acknowledged |

**State transitions** (drain): `confirming` → (confirm) → `in-progress` (streaming `DrainProgress`
events) → `complete` (all non-blocked pods evicted) | `cancelled` (operator cancelled mid-drain,
partial progress retained per spec edge case) | `failed` (e.g. bare-pod pre-check refused the
drain before any eviction started).

## Custom Resource Definition Catalog Entry (backend-held, refreshed on demand/context switch)

One entry per installed CRD, discovered via `kube::discovery` against `apiextensions.k8s.io`.

| Field | Type | Notes |
|---|---|---|
| `group` | `string` | e.g. `source.toolkit.fluxcd.io` |
| `kind` | `string` | e.g. `GitRepository` |
| `plural` | `string` | API plural name, used to build the dynamic `Api<DynamicObject>` |
| `versions` | `string[]` | Served versions; the newest/first-listed is used for discovery |
| `scope` | `'Namespaced' \| 'Cluster'` | Drives whether instance listing is per-namespace or cluster-wide |
| `permissionDenied` | `boolean` | Set if listing this CRD's instances returned a 403; surfaced per-CRD (FR-019), never blocking the rest of the catalog |

## Custom Resource Instance (backend-held, read-only this feature)

A single object of a discovered CRD kind, represented as `DynamicObject`'s
`{ types: TypeMeta, metadata: ObjectMeta, data: serde_json::Value }` shape — the same
`raw_json`-carrying shape every built-in kind's read event already uses, so it flows through the
existing generic Details/YAML/Events Inspector unchanged.

| Field | Type | Notes |
|---|---|---|
| `apiVersion` / `kind` | `string` | From `TypeMeta` |
| `namespace?` / `name` | `string` | From `ObjectMeta` |
| `resourceVersion` | `string` | Carried through for consistency with built-in kinds, though CRD-instance apply/edit is out of scope for this feature (spec Assumptions) |
| `raw_json` | `serde_json::Value` | The full object, rendered via the existing `jsonToYaml()` for the YAML tab |

**Empty/unavailable states** (spec FR-018): a cluster with zero installed CRDs shows an explicit
empty state, not an error; a CRD deleted while its instance list is open shows a "no longer
available" state rather than a stale or erroring list.

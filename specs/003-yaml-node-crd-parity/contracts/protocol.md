# Protocol Contract: v1 additions for Resource Editing, Node Actions & CRD Browsing

All variants below are additive to `src/protocol/v1/{cmd,event,mutation}.rs` and follow the exact
shape of existing analogous variants (see research.md R4). Every command variant must get an
explicit, non-wildcard arm in `mutation.rs`'s `mutation_target()` and `section()` matches; every
command and event variant must get a serialization round-trip test in `tests/protocol_serde.rs`
(constitution Principle III).

## Commands (`BackendCmdV1`)

### `ApplyResourceYaml`
```
ApplyResourceYaml {
    api_version: String,
    kind: String,
    namespace: Option<String>,
    name: String,
    yaml: String,               // full edited object text
    expected_resource_version: String,
}
```
`mutation_target()`: `Namespaced { namespaces: [namespace] }` if `namespace.is_some()`, else
`ClusterScoped`. Backend rejects (without contacting the cluster) if the parsed YAML's
`apiVersion`/`kind`/`namespace`/`name` don't match the command's own fields (FR-005) — this makes
identity-change rejection independent of any cluster round trip. On success emits
`StreamEvent::ResourceApplied`; on 409 conflict, `CommandFinished { ok: false, error }` with the
error text prefixed so the frontend can classify it as a conflict (vs. any other failure) without
string-matching against arbitrary server text elsewhere.

### `CordonNode` / `UncordonNode`
```
CordonNode { name: String }
UncordonNode { name: String }
```
`mutation_target()`: `ClusterScoped`.

### `DrainNode`
```
DrainNode {
    name: String,
    ignore_daemonsets: bool,
    force_delete_empty_dir: bool,
}
```
`mutation_target()`: `Namespaced { namespaces: [] }` is wrong (drain touches pods across multiple
namespaces) — classified `ClusterScoped`, consistent with the node itself being the target and
matching how `SelectNode`/node reads are already scoped. Starts the background eviction task;
progress streams via `StreamEvent::DrainProgress` events keyed by a request id, terminated by
`StreamClosed`.

### `CancelDrainNode`
```
CancelDrainNode { name: String, request_id: String }
```
`mutation_target()`: `ClusterScoped`. Aborts the in-flight drain task.

### `TaintNode`
```
TaintNode {
    name: String,
    op: TaintOp,               // Add | Remove
    key: String,
    value: Option<String>,
    effect: String,             // NoSchedule | PreferNoSchedule | NoExecute
}
```
`mutation_target()`: `ClusterScoped`.

### `RefreshCrds`
```
RefreshCrds
```
`mutation_target()`: `ReadOnly`. Triggers a fresh `kube::discovery` scan; emits
`SystemEvent::CrdCatalogUpdated`.

### `SelectCrdKind`
```
SelectCrdKind { group: String, kind: String, version: String }
```
`mutation_target()`: `ReadOnly`.

### `RefreshCrdInstances`
```
RefreshCrdInstances { group: String, kind: String, version: String, namespace: Option<String> }
```
`mutation_target()`: `ReadOnly`. Emits `SystemEvent::CrdInstancesUpdated`; a 403 from the cluster
is captured per-CRD (sets `permissionDenied` on the catalog entry, FR-019) rather than propagated
as a command failure that would block the rest of the catalog.

### `SelectCrdInstance`
```
SelectCrdInstance { group: String, kind: String, version: String, namespace: Option<String>, name: String }
```
`mutation_target()`: `ReadOnly`. Emits the same generic resource-read event shape (`raw_json`)
every built-in kind's select command already emits, so the existing Inspector renders it unchanged.

## Events (`StreamEvent` / `SystemEvent`)

```
StreamEvent::ResourceApplied { request_id: String, resource_version: String }
StreamEvent::DrainProgress {
    request_id: String,
    total_pods: u32,
    evicted: Vec<String>,
    blocked_on: Vec<(String, String)>,   // (pod name, verbatim server reason)
    ignored_daemonset_pods: Vec<String>,
}
StreamEvent::NodeActionFinished { request_id: String, ok: bool, error: Option<String> }

SystemEvent::CrdCatalogUpdated { entries: Vec<CrdCatalogEntry> }
SystemEvent::CrdInstancesUpdated { group: String, kind: String, version: String, namespace: Option<String>, instances: Vec<serde_json::Value> }
```

`CrdCatalogEntry` matches the data-model.md "Custom Resource Definition Catalog Entry" shape
exactly (`group`, `kind`, `plural`, `versions`, `scope`, `permission_denied`).

## Mutation classification summary

| Command | `mutation_target()` |
|---|---|
| `ApplyResourceYaml` | `Namespaced`/`ClusterScoped` (by target's own scope) |
| `CordonNode`, `UncordonNode`, `DrainNode`, `CancelDrainNode`, `TaintNode` | `ClusterScoped` |
| `RefreshCrds`, `SelectCrdKind`, `RefreshCrdInstances`, `SelectCrdInstance` | `ReadOnly` |

No wildcard arm is added to either match in `mutation.rs` — every one of the ten new variants gets
its own explicit line, matching the file's existing exhaustive-by-design pattern.

# Contract: Kind Cluster Provisioning (protocol v1 additions)

All types below are additions to `src/protocol/v1/` (`cmd.rs`, `event.rs`,
`bridge_types.rs`/a new small payload module), each with a `ts-rs` generated TypeScript binding
under `ui/src/protocol/generated/`, per the existing convention.

## Commands (`BackendCmdV1`)

```rust
RefreshKindPrerequisites,

RefreshKindClusters,

CreateKindCluster {
    request_id: String,
    name: String,               // validated DNS-1123 label; re-validated backend-side
    profile: KindClusterProfileId,   // "bare" | "custom_yaml"
    custom_yaml: Option<String>,     // required iff profile == "custom_yaml"
},

CancelCreateKindCluster {
    request_id: String,
},

DeleteKindCluster {
    request_id: String,
    name: String,
},
```

## Events (`StreamEvent` additions)

```rust
KindPrerequisitesUpdated {
    prerequisites: KindPrerequisites,
},

KindClustersUpdated {
    clusters: Vec<KindClusterSummary>,
},

KindProvisionProgress {
    request_id: String,
    stage: String,     // human-readable, sourced from kind's own progress output
},

KindClusterActionFinished {
    request_id: String,
    ok: bool,
    error: Option<String>,   // verbatim backend/kind error text on failure
},
```

## Mutation classification (`mutation.rs`)

- `RefreshKindPrerequisites`, `RefreshKindClusters` → `MutationTarget::ReadOnly`.
- `CreateKindCluster`, `DeleteKindCluster`, `CancelCreateKindCluster` → `MutationTarget::Local`
  (they mutate Fathom's/the host's local kind-cluster and kubeconfig-context state, not a live
  Kubernetes cluster's own objects — consistent with how port-forward registry and MCP server
  lifecycle commands are already classified as `Local`).

## Command → Event flow

**Create**:
1. Frontend shows the create form; on submit, validates name+profile client-side for inline
   UX, then opens `ConfirmDialog` with the pre-creation summary (name, profile, node count).
2. On confirm, frontend dispatches `CreateKindCluster { request_id, name, profile, custom_yaml }`.
3. Backend re-validates `name` (rejecting immediately via `CommandFinished`-style error if
   invalid or colliding — no subprocess spawned) and re-checks prerequisites are still `ready`.
4. Backend spawns `kind create cluster --name <name> [--config -]`, feeding `custom_yaml` via
   stdin when `profile == "custom_yaml"`, else a minimal generated single-node config.
5. Backend streams `KindProvisionProgress { request_id, stage }` for each meaningful line of
   kind's stderr output.
6. On process exit: backend re-reads the kubeconfig (`load_and_send_contexts`, same as
   `RefreshContexts`) and re-lists kind clusters (`KindClustersUpdated`), then emits
   `KindClusterActionFinished { request_id, ok, error }`.

**Delete**:
1. Frontend opens `ConfirmDialog` (destructive) naming the exact cluster.
2. On confirm, frontend dispatches `DeleteKindCluster { request_id, name }`.
3. Backend spawns `kind delete cluster --name <name>`.
4. On process exit: backend re-reads contexts and re-lists clusters as above, emits
   `KindClusterActionFinished`. If the deleted cluster's context was the active context,
   backend includes the fallback-context selection as part of the same `RefreshContexts`-driven
   update spec 012 already handles for a vanished context (no new fallback logic invented here
   if spec 012 already handles "active context no longer exists" — verify at implementation
   time and only add a minimal fallback if it does not).

## Testing requirements

- Serde round-trip test for every new `BackendCmdV1`/`StreamEvent` variant (adjacently-tagged
  JSON shape), per Principle III.
- Rust unit tests: name validator (valid/invalid/collision cases), kubeconfig-merge
  non-destructiveness (a fixture kubeconfig with 2+ unrelated contexts must be byte-identical
  in its unrelated entries after a simulated `kind`-style write), mutation classification for
  all 5 new commands.
- Playwright e2e (`ui/tests/e2e/kind-cluster-provisioning.spec.ts`) against the scripted fake
  backend: prerequisite-missing empty state, create confirm+progress+success flow, delete
  confirm+success flow, new cluster's context appearing selectable in the context switcher.

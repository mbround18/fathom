# Protocol Contract: Kubernetes Cluster Version Badge

## New type: `src/protocol/v1/cluster_version.rs`

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Live control-plane/kubelet version snapshot for the connected cluster (019).
/// Read-only, no historical data retained (spec FR-009).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct ClusterVersionSummary {
    /// `/version` discovery `gitVersion`, e.g. "v1.29.4". `None` when discovery failed/timed out.
    pub server_version: Option<String>,
    /// Distinct `status.nodeInfo.kubeletVersion` values across visible nodes, sorted.
    pub node_kubelet_versions: Vec<String>,
    /// True only when `server_version` is `Some` and at least one entry in
    /// `node_kubelet_versions` differs from it.
    pub skew_detected: bool,
}
```

Exported via `src/protocol/v1/mod.rs`:

```rust
mod cluster_version;
...
pub use cluster_version::*;
```

## New event variant: `src/protocol/v1/event.rs`

Added to `enum SystemEvent`, alongside `FluxOverviewUpdated`/`HubbleTrafficUpdated`:

```rust
/// Cluster + node Kubernetes version snapshot, refreshed periodically (019).
ClusterVersionUpdated(ClusterVersionSummary),
```

`ClusterVersionSummary` must be added to the `use super::{...}` import list at the top of
`event.rs`, and a matching arm added to the `all_system_events_is_exhaustive` match in
`tests/protocol_serde.rs`:

```rust
SystemEvent::ClusterVersionUpdated(..) => {}
```

## Emission site: `src/backend/sync.rs`

```rust
pub(super) async fn ensure_cluster_version_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.cluster_version_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.cluster_version_poll_task = Some(tokio::spawn(poll_cluster_version(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

async fn poll_cluster_version(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match cluster_version::fetch_cluster_version(&client).await {
            Ok(summary) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::global(section_keys::system()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::ClusterVersionUpdated(summary)),
                );
            }
            Err(e) => {
                events::notify_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background cluster version poll failed: {}", e),
                );
            }
        }
    }
}
```

`ensure_cluster_version_poll` is also called once, immediately (not just after the first 20s
tick), from `refresh_core_resources` in `src/backend/cmd/cluster.rs` via a fire-and-forget initial
fetch so the badge populates promptly on connect rather than waiting up to 20s — mirroring how
`refresh_core_resources` already does an immediate `refresh_namespaces`/`refresh_pods` alongside
starting `ensure_background_sync`'s longer-lived watchers. Concretely: `poll_cluster_version`'s
loop is restructured as "fetch once immediately, then tick every 20s" (same shape already usable
by copying the `ticker.tick().await` call to after the first fetch instead of before it), so no
separate one-shot function is needed.

## Frontend binding (generated, not hand-written)

`ui/src/protocol/generated/ClusterVersionSummary.ts` (ts-rs output):

```ts
export type ClusterVersionSummary = {
  server_version: string | null,
  node_kubelet_versions: Array<string>,
  skew_detected: boolean,
};
```

`ui/src/protocol/generated/SystemEvent.ts` gains a `{ type: "ClusterVersionUpdated", data:
ClusterVersionSummary }` arm (regenerated, not hand-edited).

## Round-trip test: `tests/protocol_serde.rs`

```rust
fn sample_cluster_version_summary() -> fathom::protocol::ClusterVersionSummary {
    fathom::protocol::ClusterVersionSummary {
        server_version: Some("v1.29.4".to_owned()),
        node_kubelet_versions: vec!["v1.28.9".to_owned(), "v1.29.4".to_owned()],
        skew_detected: true,
    }
}
```

...plus a `roundtrip(...)` call and a `SystemEvent::ClusterVersionUpdated(sample_cluster_version_summary())`
sample in the existing `system_roundtrip` test list, following the exact pattern used for
`sample_flux_overview()`.

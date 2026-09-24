# Data Model: Kubernetes Cluster Version Badge

## ClusterVersionSummary

The current, live-observed Kubernetes version state for the connected cluster. Emitted as the
payload of `SystemEvent::ClusterVersionUpdated`.

| Field | Type | Description |
|---|---|---|
| `server_version` | `Option<String>` | Control-plane `gitVersion` from `/version` discovery (e.g. `"v1.29.4"`). `None` when the discovery call failed or timed out (spec edge case: neutral "unknown" state, FR-005). |
| `node_kubelet_versions` | `Vec<String>` | Distinct kubelet version strings observed across all nodes visible to the operator's RBAC, sorted for stable ordering. Empty when no nodes are visible/loaded (spec Acceptance Scenario 4 — neither warned nor "all clear"). |
| `skew_detected` | `bool` | `true` when `node_kubelet_versions` contains at least one entry different from `server_version` (only meaningful — and only ever `true` — when both `server_version` is `Some` and `node_kubelet_versions` is non-empty). |

**Invariants**:
- `skew_detected` MUST be `false` whenever `server_version` is `None` or `node_kubelet_versions` is
  empty (there is nothing to compare) — matches spec Edge Cases ("no false warning, no false
  all-clear").
- `node_kubelet_versions` holds distinct values only (deduplicated), not one entry per node —
  keeps the payload small and matches the spec's "concise summary" edge case for large clusters.
- No historical/prior values are retained anywhere in this type (spec FR-009); each emission
  fully replaces the frontend's prior state.

**Relationships**: Derived from two independent read paths — the Kubernetes discovery `/version`
endpoint (control-plane) and `Node.status.nodeInfo.kubeletVersion` (per node) — combined
server-side into one summary so the frontend never needs to reconcile two separate events.

**State transitions**: None persisted; this is a snapshot replaced wholesale on every periodic
poll tick (20s) or context switch. The frontend's corresponding store slice simply holds "the most
recent `ClusterVersionSummary` received, or `undefined` before the first one arrives."

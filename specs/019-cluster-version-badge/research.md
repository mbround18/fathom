# Research: Kubernetes Cluster Version Badge

## R1: How to fetch the server version

**Decision**: Use `kube::Client::apiserver_version()`, already used in
`src/backend/context_status.rs` for the per-context reachability probe (`GET /version`). It
returns `k8s_openapi::apimachinery::pkg::version::Info`, whose `git_version` field (e.g.
`"v1.29.4"`) is the human-readable server version string to display.

**Rationale**: Reuses an already-proven, already-imported call path; no new dependency; already
demonstrated to work against real clusters and to time out cleanly.

**Alternatives considered**: Hitting `/version` via a raw HTTP request through `client.request(...)`
— rejected, `apiserver_version()` already wraps this correctly and is idiomatic kube-rs.

## R2: How to detect kubelet skew

**Decision**: List `Node` objects the same way `src/backend/nodes.rs::fetch_node_summaries` does
(`Api::<Node>::all(client.clone()).list(&ListParams::default())`), and read
`status.nodeInfo.kubeletVersion` per node (same field `nodes.rs::system_info()` already extracts
into `NodeDetails::kubelet_version`). Skew is "any node's kubelet version string differs from the
control-plane server version string" (per spec Assumptions — no semantic version parsing beyond
string comparison is required for v1).

**Rationale**: Reuses the existing, RBAC-respecting node-listing path; no new permissions
requested (Constitution VI). String comparison is sufficient per the spec's explicit assumption,
avoiding a semver-parsing dependency for a low-risk, informational feature.

**Alternatives considered**: Adding `kubelet_version` to the existing `NodeSummary`/`NodesUpdated`
event and deriving skew in the frontend from the already-streamed node list — rejected because (a)
`NodeSummary` is emitted on a different cadence/trigger (pod-count-aware) than a lightweight
version poll needs, (b) it would require a protocol change to a widely-used existing type and
recomputing skew in every frontend consumer of node data, and (c) the Flux/Hubble precedent already
establishes a self-contained periodic "scan and summarize" background task — the cheaper, better
precedent-aligned option is a dedicated small fetch (like `flux::scan_flux`) that lists nodes
itself, independent of the node-summary refresh cycle.

## R3: Where the background poll fits into the connect/refresh lifecycle

**Decision**: Add `ensure_cluster_version_poll(state, sender)` to `src/backend/sync.rs`, called
from `refresh_core_resources()` in `src/backend/cmd/cluster.rs` alongside
`ensure_flux_poll`/`ensure_hubble_poll`. It spawns `poll_cluster_version` (mirrors
`poll_flux_overview` exactly): a 20s `tokio::time::interval` loop that calls a new
`cluster_version::fetch_cluster_version(&client)` and emits
`SystemEvent::ClusterVersionUpdated(summary)` on success, or `events::notify_error(...)` on
failure (leaving the last-known badge state in place in the UI rather than clearing it — matches
existing Flux/Hubble poll error handling, and satisfies FR-005's "don't show stale/incorrect data"
by surfacing the error via the existing notification channel while the frontend can choose to mark
the last value stale). The task handle lives in `WatchTasks` (`cluster_version_poll_task` field)
and is aborted/restarted by the existing `WatchTasks::abort_all()` on context switch/disconnect —
this is what prevents a stale badge from a previous cluster surviving a context switch (spec edge
case).

**Rationale**: Exactly matches an established, reviewed pattern (`ensure_flux_poll`) with no new
lifecycle concept introduced. Reuses `abort_all()` for free correctness on context switch.

**Alternatives considered**: A one-shot fetch only on connect (no periodic refresh) — rejected,
spec FR-002 explicitly requires periodic refresh consistent with other lightweight cluster facts,
and cluster upgrades/node replacements can happen during a long-running session.

## R4: Initial (first-render) state before the poll completes

**Decision**: The frontend badge renders an "unknown" neutral state (a muted pill, no skew
warning) until the first `ClusterVersionUpdated` event arrives, exactly as `FluxOverviewUpdated`
consumers do (no overview until the first event lands). No synchronous/blocking command is added
to fetch this on shell mount — it arrives via the same event-driven path as everything else in
`refresh_core_resources`, satisfying FR-006 (must not block/delay app-shell render).

**Rationale**: Matches Constitution V's "don't block rendering" and the existing async/event-driven
UI-population pattern used everywhere else in the app.

**Alternatives considered**: A synchronous `LoadClusterVersion` command sent by the frontend on
mount — rejected as an unnecessary new command when the existing periodic-refresh-on-connect flow
already covers it, per the plan's mandate to reuse `sync.rs`'s established pattern.

## R5: Route/section key for the event

**Decision**: Reuse the existing `Route::global(section_keys::system())` used for
`ContextChanged`, since the version badge is a global, always-visible shell element, not scoped to
one resource section — matching how `ContextChanged`/`ContextsUpdated` are routed rather than
`section_keys::integrations_flux()` (which is scoped to a specific integrations panel/section the
version badge does not have).

**Rationale**: The badge lives in the app shell (not a dedicated panel/section), so a global system
route is the correct fit; avoids inventing an unused new `SectionKey` for a feature with no
dedicated panel route.

**Alternatives considered**: A new `section_keys::cluster_version()` — rejected as unnecessary
since there is no dedicated panel/section this event is scoped to; `system()` already exists for
exactly this "global shell fact" purpose.

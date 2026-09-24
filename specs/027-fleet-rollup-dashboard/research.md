# Phase 0 Research: Fleet Rollup Dashboard

## R0: Current client-lifecycle model — what actually exists today

**Finding** (read directly from `src/backend/state.rs`, `src/backend/cmd/cluster.rs`,
`src/backend/context_status.rs`, and `specs/012-incluster-daemon-federation/data-model.md` /
`research.md`):

- `BackendState.cluster` (`src/backend/state.rs::ClusterContext`) holds exactly **one**
  `client: Option<kube::Client>` plus one `current_context: Option<String>`. There is no
  `HashMap<ContextId, Client>` anywhere in the codebase.
- All long-running background work — `WatchTasks` (pod/deployment watches, namespace sync,
  Hubble poll, Flux poll), the metrics poller, exec sessions, log-tailing — is similarly
  singular: one `Option<JoinHandle<_>>` per kind, implicitly scoped to "whatever the active
  context is right now."
- `cmd/cluster.rs::set_context()` switches contexts by **tearing down every one of those
  tasks** (`log_stream.stop()`, `exec_session.stop()`, `watch_tasks.abort_all()`,
  `node_actions.abort_all()`, `metrics.reset_probe()`), reconnecting a brand-new
  `kube::Client` via `connect_and_share_client()`, and re-running `refresh_core_resources()`.
  This is a hard "one cluster live at a time" model, not a resource-pooling one.
- Spec 012 (the federation/multi-context switcher) explicitly **did not** solve concurrent
  liveness. Its `data-model.md` says outright: *"Only one context is ever `Connected` at a
  time in P1 (the single active-client model, unchanged)... P2 will need to revisit it when
  several contexts can be simultaneously live."* Its `research.md` (R2) explicitly rejected a
  periodic poller and (R1) deliberately built only a **short-lived, one-shot** `kube::Client`
  per non-active context (`context_status.rs::probe_context`), torn down immediately after a
  single `apiserver_version()` call, specifically to keep steady-state idle CPU at zero
  (Constitution V) and to avoid holding N live clients at once.

**Conclusion — be honest about scope**: This feature is *not* a small extension of an
existing multi-client design; spec 012 intentionally avoided building one. What spec 012 *does*
give us to build on is the pattern for constructing a client for an arbitrary named context
(`load_kubeconfig` + `Config::from_custom_kubeconfig` + `KubeConfigOptions{ context: Some(name) }`
+ `Client::try_from`, in `context_status.rs::probe_context`) and the existing
per-task-kind `Option<JoinHandle>` / `HashMap<String, JoinHandle>` lifecycle-management
idioms already used for drain tasks (`NodeActionState.drain_tasks`) and watch tasks
(`WatchTasks`). The fleet dashboard reuses those idioms, generalized to be *keyed by context*
and *recurring* instead of one-shot — it does not touch or repurpose `ClusterContext.client`
(the single active client), avoiding any risk to the existing single-context UX or its P1
race-avoidance invariant (research.md R3 of spec 012).

## R1: Where the new concurrent client pool lives

**Decision**: Add a new `FleetState` to `BackendState` (`src/backend/state.rs`), holding:
- `pollers: HashMap<String, tokio::task::JoinHandle<()>>` — one recurring poll task per
  *connected* context, keyed by context name (mirrors `NodeActionState.drain_tasks`'s
  keyed-by-id pattern, not `WatchTasks`'s singular-slot pattern, because the fleet dashboard's
  cardinality is per-context, not per-kind).
- `summaries: HashMap<String, FleetContextSummary>` — last-known summary per context, so a
  freshly opened dashboard can render immediately from whatever has already been polled
  rather than waiting a full interval, and so a UI reconnect/re-render doesn't need to
  re-fetch.
- Each poller owns its **own** `kube::Client`, built with the same
  `load_kubeconfig`/`Config::from_custom_kubeconfig`/`KubeConfigOptions{context}` construction
  `context_status::probe_context` already uses — but, unlike that one-shot probe, the client is
  held for the poller task's lifetime and reused across polls (avoids rebuilding TLS/exec-auth
  state every interval).

**Rationale**: Keeps the new concurrent-client concept fully isolated from `ClusterContext`
(no risk of the fleet poller's client leaking into or racing with the single active-context
client that every existing resource module assumes) while reusing proven construction code.
A keyed `HashMap` of task handles is already an established, tested pattern in this codebase
(`NodeActionState`), so this is idiomatic, not novel.

**Alternatives considered**:
- Extending `ClusterContext.client` into `HashMap<String, Client>` and updating every
  resource module (`pods.rs`, `deployments.rs`, etc.) to take a context parameter — rejected:
  massive blast radius across ~40 existing resource modules, all of which assume "the" client;
  wildly out of proportion to a view-only rollup feature, and explicitly against the
  spec's non-goal of not touching the single-context UX.
- Reusing `context_status.rs`'s one-shot probe machinery unmodified and just polling it more
  often — rejected: it only proves reachability (`apiserver_version()`); the fleet dashboard
  needs node counts, pod health, and Flux state, i.e. real API calls (`Api::<Node>::list`,
  `Api::<Pod>::list`, Flux CRD lists) that are too expensive to *rebuild a client and pay full
  TLS/exec-auth handshake cost* every poll — a persistent-per-context client amortizes that.

## R2: Lifecycle — when pollers start/stop (Constitution V idle-CPU budget)

**Decision**: Fleet pollers are **only** alive while the Fleet Dashboard view is open on the
frontend, not for the app's whole lifetime. The frontend sends a new `StartFleetPolling` /
`StopFleetPolling` command pair (mirroring the existing "start on view-enter, stop on
view-exit" pattern used by `SetActiveSection`'s light-prefetch and by log/exec sessions'
explicit stop calls) when the dashboard view mounts/unmounts. `StartFleetPolling` spawns one
poller per currently-connected context (from `session.contexts` filtered to `Connected` in
`contextStatuses`, mirroring spec 012's status model) plus a listener so that a context
connecting/disconnecting while the dashboard is open starts/stops its poller incrementally
(FR-006). `StopFleetPolling`, and backend shutdown/`cleanup_all_tasks`, abort every handle in
`pollers` (same `abort_task` helper already used everywhere else in `state.rs`).

**Rationale**: Directly honors Constitution V ("an idle app MUST use under 2% of one CPU
core... redrawing continuously when nothing has changed is a defect") — an operator who never
opens the Fleet Dashboard pays zero extra cost, and N background pollers times N clusters is
exactly the kind of steady-state cost spec 012's research.md R2 already flagged as something
"better designed deliberately if/when P2's sustained multi-connection model lands." This is
that deliberate design point.

**Alternatives considered**:
- Always-on fleet polling from app startup regardless of whether the dashboard is open —
  rejected: directly violates Constitution V for the common case of operators who don't use
  the fleet view, and multiplies idle resource cost (sockets, memory, periodic wake-ups) by
  the number of connected contexts for no benefit.
- Polling triggered by explicit manual refresh only (no interval) — rejected: fails spec
  User Story 3 / FR-004 ("continue to poll... on a recurring interval... while the dashboard
  is open") and SC-004's staleness bound.

## R3: Poll interval, timeout, and per-context failure isolation

**Decision**: 30-second poll interval per context (consistent with the "tens of seconds"
default the spec's Assumptions section calls out), each individual context's fetch (node
list + pod list + version + optional Flux overview) wrapped in `tokio::time::timeout` at 10s
total, matching the existing 5s-per-probe precedent in `context_status.rs` scaled up for the
larger amount of data fetched. A context's poller loop is a plain `loop { poll_once();
sleep(30s) }` task, isolated by construction — a panic or error in one context's poll only
affects that context's summary entry (caught and turned into an `Error` status on that
`FleetContextSummary`, never propagated to other pollers or crashes the task), directly
satisfying FR-004/FR-010 and Edge Cases (slow/unreachable context, expired credentials).

**Rationale**: Each poller is an independent `tokio::spawn`'d task already, so isolation is
free from Rust/tokio's task model — no explicit supervisor is needed, just disciplined
error-to-status mapping inside each loop iteration (reusing `ContextConnectionStatus`-style
states, extended with an `AuthError`/`Unreachable` distinction per FR-005).

**Alternatives considered**: A single task that round-robins all contexts sequentially —
rejected: violates FR-004's "MUST NOT block or delay other contexts" and SC-003 directly,
since one slow cluster would delay every other cluster's update by construction.

## R4: GitOps/Flux status reuse

**Decision**: Reuse the existing `FluxOverview`/`FluxResourceSummary` types
(`src/protocol/v1/flux.rs`) and the existing Flux-detection/list logic already used by
`sync.rs::poll_flux_overview` for the single active context, calling the same detection code
against each fleet poller's own per-context client. Reduce the full `FluxOverview` down to a
single rollup status (`Synced | OutOfSync | NotInstalled`) for the fleet card, per FR-003.

**Rationale**: Flux support and its CRD-detection/backoff-for-missing-CRDs handling already
exist and are tested; re-deriving them per context is a parameter change, not new logic.

**Alternatives considered**: Building a new lightweight Flux-presence check — rejected as
needless duplication of already-correct, already-tested code.

## R5: Frontend surface

**Decision**: New route/section `fleet` in the existing `PaneHost`/shell system, added to
`section_keys` and the nav, listed as its own top-level dashboard rather than folded into an
existing per-context view. New Zustand store slice `fleetSummaries: Record<string,
FleetContextSummary>` populated by a new `FleetSummaryUpdated` event handler, following the
exact pattern `contextStatuses` already uses (`ui/src/store/handlers/system.ts`) — last
write per context wins, not persisted, re-populated on each mount.

**Rationale**: Matches every existing precedent in this codebase for "list of per-context
state, updated by discrete backend events, rendered as a strip/grid of cards"
(`FederationContextSwitcher.tsx` is the direct visual/structural precedent), and keeps this
feature additive rather than modifying the existing single-context dashboard or
`FederationContextSwitcher`.

**Alternatives considered**: Reusing `FederationContextSwitcher`'s strip itself to also show
health data — rejected per the spec's own framing (that switcher's job is a quick-switch
affordance across the whole app chrome; overloading it with rollup data would violate its
existing, tested "narrow strip" layout and its "augment don't replace" precedent from spec
012's research.md R4).

## Clarification update (2026-09-24)

- **Fan-out (FR-012)**: one poller per context still, but each poll holds a permit from a shared 8-permit semaphore and the first poll is offset by `index x 250 ms` (cap 5 s) to avoid a start burst.
- **Partial RBAC (FR-013)**: nodes, pods and Flux are fetched independently; a kube 403 on nodes/pods marks that group `restricted` instead of failing the poll. Auth classification matches typed `kube::Error` (`Api` 401/403, `Auth`) instead of message substrings.
- **Unhealthy pods (FR-014)**: extended to stuck-waiting containers and Pending > 5 min (see data-model.md).
- **Open risk**: a fresh kube client is still built every poll (exec-plugin cost); revisit with client caching if it shows up in practice.
- **Known behavior change**: a plain-text "401 Unauthorized" error that is not a typed kube error now maps to `Unreachable`.

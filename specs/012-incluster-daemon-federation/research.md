# Phase 0 Research: P1 Multi-Context Switcher & Live Status

## R1: How to determine per-context reachability without a live client

**Decision**: Build a short-lived `kube::Client` per non-active context from the already-loaded
`Kubeconfig` (same `Config::from_custom_kubeconfig` + `KubeConfigOptions{ context: Some(name) }` path
`connect_client_for_current_context` already uses), then call `client.apiserver_version()` wrapped in a
`tokio::time::timeout(Duration::from_secs(5), ...)`. Success -> reachable; timeout or error -> unreachable,
with the error's `Display` surfaced verbatim (Principle I: backend errors reach the operator verbatim).

**Rationale**: `apiserver_version()` is the cheapest authenticated round-trip already available on `kube::Client`
(a GET to `/version`), requires no RBAC beyond what any authenticated user has, and exercises the exact
same auth path (including any `exec` credential plugin) a real connection would use — so a probe failure
is a true signal, not a false negative. Reuses 100% existing dependencies.

**Alternatives considered**:
- TCP-connect to the API server host:port only — rejected: doesn't exercise auth, so an unreachable-because-
  bad-credentials context would falsely report "reachable".
- Reusing `kube-rs`'s `Client::try_default()` discovery cache — rejected: that resolves the *default*
  context only, not an arbitrary named context.
- A persistent watch per context (P2's approach) — explicitly deferred; P1 does not need or want a
  sustained connection per context, only a point-in-time reachability signal.

## R2: Probe lifecycle vs. Constitution V (idle CPU budget)

**Decision**: Probes are **one-shot**, fired only when the context list is (re)loaded (`Initialize`,
`RefreshContexts`, `SetContext`'s post-switch reload), each bounded to 5s by `tokio::time::timeout`, and
never retried automatically. No new periodic timer/interval is introduced.

**Rationale**: Matches existing patterns in the codebase (e.g. `metrics::probe_metrics_api` is also a
one-shot best-effort probe, not a poller) and keeps idle CPU at zero once probes settle — there is no
steady-state cost.

**Alternatives considered**: A periodic re-probe interval (e.g. every 30s) — rejected for P1 as unnecessary
scope creep beyond FR-002's "live status... without requiring the operator to manually refresh", which is
satisfied by re-probing on the existing kubeconfig-reload cadence (FR-005) plus explicit switches; a timed
poller is exactly the kind of "redrawing/working when nothing changed" Constitution V warns against, and
is better designed deliberately if/when P2's sustained multi-connection model lands.

## R3: Active-context status vs. background-probe status — avoiding a race

**Decision**: The active context is **never** included in the set of contexts background-probed. Its
status is instead driven explicitly and synchronously by the code path that already connects it
(`initialize()` / `set_context()` in `src/backend/cmd/cluster.rs`): emit `Connecting` immediately before
`connect_and_share_client`, then `Connected` or `Error{detail}` immediately after, in the same function.

**Rationale**: Both a background probe and the authoritative connect attempt racing to set the same
context's status is a real hazard (whichever completes last wins, non-deterministically). Excluding the
active context from probing removes the race entirely and is strictly more accurate — the active context's
status comes from the actual connection outcome, not a proxy probe.

**Alternatives considered**: Sequence/seq-number based "latest wins" reconciliation on the frontend —
unnecessary complexity; the backend already assigns a monotonic `seq` per envelope that the existing
store/ingest pipeline uses for staleness, but avoiding the double-write at the source is simpler and
matches how `ContextChanged` already works (single authoritative writer).

## R4: Frontend placement — "persistent, always-visible list" without breaking existing tests

**Decision**: Add a new component, always mounted in `AppShell` directly below `TopBar` (not inside any
menu/dropdown/sheet), showing every context as a row with a status dot. The existing `ContextPicker`
combobox in `TopBar` is left untouched (it remains a valid, already-tested quick-switch affordance).

**Rationale**: FR-001 requires the list to not be "a menu the operator must open to discover contexts
exist" — an always-mounted strip satisfies this without touching `ContextPicker`, so
`ui/tests/e2e/us1-context-switch.spec.ts` (which asserts on `getByRole('combobox', { name: 'Context' })`)
keeps passing unmodified, honoring "augment, don't replace."

**Alternatives considered**: Replacing `ContextPicker` outright — rejected per the task brief's explicit
"augmenting" guidance and to avoid destabilizing already-shipped, tested UI; folding the list into the
`NavTree` drawer — rejected because on tablet/desktop widths the drawer is not "always visible" (it is a
`Sheet`), which would fail FR-001 at those widths.

## R5: Status enum shape

**Decision**: `ContextConnectionStatus` fieldless enum (`Connected | Connecting | Error | NotConnected`,
serialized as a plain string via existing `ts-rs`/serde conventions, mirroring `NotificationLevel`), paired
with a `SystemEvent::ContextStatusUpdated { context: String, status: ContextConnectionStatus, error:
Option<String> }` event. `error` is populated only when `status == Error`.

**Rationale**: Matches spec.md's Key Entities ("Cluster Connection... status..., an optional last-error
detail") and FR-002's minimum four states exactly; fieldless-enum-as-string is the established convention
in this codebase (`NotificationLevel`).

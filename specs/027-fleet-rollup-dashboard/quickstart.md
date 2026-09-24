# Quickstart: Fleet Rollup Dashboard

## Prerequisites

- Fathom built from this branch (`027-fleet-rollup-dashboard`).
- At least two kubeconfig contexts reachable (for manual validation; two `kind` clusters is
  the fastest local setup — see existing `scripts/` for any cluster-bootstrap helpers used by
  other specs' quickstarts, e.g. spec 012's).
- One of the two clusters should have at least one pod in a non-Running/non-Ready state to
  exercise the "degraded" card (e.g. `kubectl run bad --image=does-not-exist:latest`).
- Optional: Flux installed on one cluster (`flux install`) with at least one failing
  Kustomization, to validate the GitOps rollup indicator (User Story 2).

## Manual validation

1. Launch Fathom (`cargo tauri dev` or the packaged app) with both contexts present in
   `~/.kube/config` and both reachable.
2. Confirm the existing `FederationContextSwitcher` strip shows both contexts as `Connected`
   before opening the Fleet Dashboard (baseline: spec 012 behavior is unaffected).
3. Open the Fleet Dashboard view from the nav.
4. **Expect**: within a few seconds, a card renders for each connected context, showing node
   ready/total count, unhealthy-pod count, and cluster version — without the active context
   (per the `FederationContextSwitcher`/title bar) changing.
5. **Expect**: the context with the bad pod shows a "degraded" indicator; the healthy one shows
   "healthy."
6. If Flux is installed on one context: **expect** that context's card shows a Flux sync
   indicator ("out of sync" for the failing Kustomization); the other context's card shows no
   Flux indicator (or "not applicable"), not an error.
7. Wait one poll interval (30s). **Expect**: card data timestamps/values refresh without any
   user action.
8. Click a card. **Expect**: Fathom switches its active context to that card's cluster and
   navigates to the normal single-context dashboard for it (the existing `SetContext` flow).
9. Reopen the Fleet Dashboard, then disconnect one context via the existing federation
   switcher UI (or make its cluster unreachable). **Expect**: that context's card either
   disappears (on disconnect) or transitions to an "unreachable" status (on unreachability)
   within one poll cycle, without disrupting the other context's card.
10. Close the Fleet Dashboard view (navigate away). **Expect** (backend-observable via logs/
    `cargo tauri dev` console): fleet pollers stop — no continued polling traffic to either
    cluster while the view is closed (Constitution V — see research.md R2).

## Automated validation

- `cargo test` — unit tests for `src/backend/fleet.rs` poll-loop error-to-`FleetHealthState`
  mapping, `FleetState` start/stop idempotency, and the `error`-only-on-failure invariant
  (mirrors `context_status.rs`'s existing test style).
- `tests/protocol_serde.rs` — round-trip tests for `FleetContextSummary`, `FleetHealthState`,
  `FluxRollupStatus`, `StartFleetPolling`/`StopFleetPolling`, `FleetSummaryUpdated`.
- `pnpm -C ui test` — Vitest unit tests for the `fleetSummaries` store slice/handler.
- `ui/tests/e2e/us1-fleet-dashboard.spec.ts` — one Playwright scenario against the scripted
  fake backend: two fixture contexts (one healthy, one with an unhealthy pod) already
  connected; open the Fleet Dashboard; assert both contexts' cards render with correct health
  status; assert the active context (per the store) is unchanged throughout.

See [data-model.md](./data-model.md) for the exact field shapes and
[contracts/fleet-protocol.md](./contracts/fleet-protocol.md) for the command/event wire format
referenced above.

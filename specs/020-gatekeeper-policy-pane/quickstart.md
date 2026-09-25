# Quickstart: Validating the Gatekeeper Policy Pane

## Prerequisites

- A kube-context Fathom can connect to.
- For the "installed with violations" path: OPA Gatekeeper installed (e.g.
  `kubectl apply -f https://raw.githubusercontent.com/open-policy-agent/gatekeeper/master/deploy/gatekeeper.yaml`),
  one `ConstraintTemplate` applied, and one `Constraint` of that template's kind applied against
  a namespace/resource that violates it (e.g. a `K8sRequiredLabels` template requiring a
  `team` label, and a `Constraint` targeting `Namespace` with an existing namespace missing that
  label).
- For the "not installed" path: any cluster without Gatekeeper's CRDs (a fresh kind/minikube
  cluster works).

## Scenario 1: Live constraint list + violation drill-down

1. Point Fathom at the cluster with Gatekeeper and the violating constraint above.
2. Open the Gatekeeper pane (`integrations/gatekeeper`).
3. **Expected**: the constraint appears in the list within one poll cycle (≤20s, or immediately
   via the pane's own `RefreshGatekeeper` on open) showing its kind, enforcement action, and a
   nonzero violation count sourced from `.status.totalViolations`.
4. Select the constraint.
5. **Expected**: the violating resource (the namespace from the setup above) appears with its
   namespace/kind/name and Gatekeeper's violation message.
6. Click the violating resource.
7. **Expected**: the app navigates to the Inspector, showing that exact object.
8. Resolve the violation in the cluster (e.g. add the missing label) and wait for the next poll
   tick.
9. **Expected**: the violation count drops (or the constraint moves to a "0 violations" state)
   without manual refresh.

This scenario is covered end-to-end by `ui/tests/e2e/gatekeeper-panel.spec.ts` against a scripted
fake backend (per Constitution Principle III) — no live cluster is required to run the automated
test; the steps above are for manual/exploratory validation against a real cluster.

## Scenario 2: Honest "not installed" state

1. Point Fathom at a cluster with no Gatekeeper CRDs.
2. Open the Gatekeeper pane.
3. **Expected**: an explicit "OPA Gatekeeper is not installed on this cluster" message, not a
   spinner, blank list, or generic error.

Also covered by the automated Playwright spec (the second of its two scenarios).

## Validation checklist

- [ ] `cargo test -p fathom` passes, including the new `src/protocol/v1/gatekeeper.rs` round-trip
      tests and `src/backend/gatekeeper.rs`'s unit tests for summarization/truncation logic.
- [ ] `cargo test -p fathom scan_gatekeeper_against_live_cluster -- --ignored --nocapture` (manual,
      real-cluster-only, mirrors `flux.rs`'s equivalent `#[ignore]` test) succeeds against a
      cluster with Gatekeeper installed.
- [ ] `pnpm -C ui test` passes, including new unit coverage for the `gatekeeper` store slice and
      `GatekeeperPanel.tsx`'s rendering logic (awaiting-status, truncation notice, empty states).
- [ ] `pnpm -C ui exec playwright test gatekeeper-panel.spec.ts` passes both scenarios.
- [ ] `cargo clippy --all-targets -- -D warnings` and `pnpm -C ui lint` are clean.

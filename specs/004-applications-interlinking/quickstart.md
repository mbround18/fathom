# Quickstart: Applications View & Rich Interlinking

## Prerequisites

- A reachable cluster (kind/minikube/real) with at least:
  - one Helm-installed release (e.g. `helm install redis bitnami/redis -n cache --create-namespace`)
  - Flux installed with at least one `Kustomization` reconciling a workload, and ideally one
    `HelmRelease` (any Flux quickstart bootstrap suffices; not required if only validating the
    Helm-only path)
- `cargo build` and `pnpm -C ui build` succeed on a clean checkout (baseline, unrelated to this
  feature).

## Scenario 1 — Applications list (User Story 1)

1. Launch Fathom, connect to the cluster above.
2. Open the new "Applications" section in the left nav.
3. **Expect**: a row for the Helm release (`Managed By: Helm`) and a row for the Flux
   `Kustomization`/`HelmRelease` (`Managed By: Flux`), each with correct namespace, version, age,
   and an aggregated status reflecting their owned workloads' readiness.
4. Narrow the namespace filter to exclude one of the two namespaces.
5. **Expect**: the excluded application's row disappears, matching every other list in Fathom.

Validates: spec.md FR-001–FR-005, SC-001, SC-003.

## Scenario 2 — Application inspector (User Story 2)

1. From the list in Scenario 1, select the Flux-managed application.
2. **Expect**: the Inspector opens in place (list pane stays visible), showing creation time,
   status, version/revision, managed-by, and any derived URLs.
3. **Expect**: a metrics area shows aggregated CPU/memory across the application's pods (or an
   explicit "unavailable" state if no Prometheus source is configured — verify both states by
   toggling the Prometheus integration off/on if convenient).
4. **Expect**: a related-resources list shows the application's Pods (with readiness/CPU/memory),
   Deployments/StatefulSets, ServiceAccounts, and any CRD instances it owns.

Validates: spec.md FR-006–FR-008, SC-002.

## Scenario 3 — Drill-down and back-stack (User Story 3)

1. From the application inspector in Scenario 2, click one of its related Pods.
2. **Expect**: the Pod's own inspector opens (Details/YAML/Events/Logs/Terminal tabs, as already
   exist for Pods) and a breadcrumb shows the drill-down path (application → pod).
3. From the Pod inspector, click its Node (via the Pod's `scheduled-on` relation).
4. **Expect**: the Node's inspector opens, breadcrumb now shows three steps.
5. Click "back" twice.
6. **Expect**: first lands on the Pod inspector (restored, not re-fetched from blank), then on the
   Application inspector; the back control is disabled once no prior step remains.
7. Separately, open a Node inspector directly from the Nodes list and confirm its related Pods
   (those scheduled on it) are clickable; open a Service inspector and confirm its selected Pods
   are clickable; open a PVC inspector and confirm its mounting Pod(s) are clickable.
8. Click a related Pod link, then immediately delete that pod out-of-band (`kubectl delete pod ...`)
   before it's clicked, then click the (now-stale) link.
9. **Expect**: a clear "not found" state in the target inspector, not a blank or frozen panel.

Validates: spec.md FR-009–FR-015, SC-004, SC-005.

## Scenario 4 — Edge cases

1. Create a workload with neither Helm nor Flux ownership metadata (a hand-applied manifest with
   no `app.kubernetes.io/*` labels at all).
2. **Expect**: it does not appear in the Applications list, but still appears in the existing
   Workloads views, unchanged.
3. Delete every resource a Helm release once owned, out of band, without uninstalling the release
   marker resources Fathom used to identify it (or simulate by picking a release with zero
   remaining owned workloads).
4. **Expect**: the application still appears in the list with a status indicating no active
   workloads (FR-016), not hidden or erroring.

Validates: spec.md Edge Cases, FR-016.

## Automated coverage (per Constitution Principle III)

- `tests/protocol_serde.rs`: round-trip test for every new command/event variant and new shared
  type listed in `contracts/protocol.md`.
- A scripted-fake-backend E2E test per user story (US1: list renders and respects namespace
  filter; US2: inspector shows metadata/metrics/related list; US3: drill-down + back-stack + stale
  link "not found" state), following the existing UI-driven-against-fake-backend convention used
  for `002`/`003`'s E2E suites.
- Unit tests for the related-resources resolver's relationship vocabulary (data-model.md's table)
  and for Helm/Flux/label-heuristic attribution precedence (research.md R1–R3), mirroring existing
  unit-test conventions in `flux.rs`/`metrics.rs`.

# Quickstart: Certificates (cert-manager) Pane

## Prerequisites

- A Kubernetes cluster reachable via the active kubeconfig context.
- For the full happy path: cert-manager installed (`kubectl get crd certificates.cert-manager.io`)
  with at least one Certificate close to its expiry threshold and one Certificate stuck at a
  Challenge (e.g. a HTTP-01 ACME challenge that cannot be solved because no ingress controller
  routes the `.well-known/acme-challenge` path).
- For the not-installed path: any cluster without cert-manager's CRDs.

## Manual validation

1. `cargo build --workspace` then run Fathom against a cluster with cert-manager installed.
2. Open the Certificates pane (tool pane, alongside Flux/Hubble).
3. Confirm every Certificate appears with name, namespace, issuer reference, and an expiry
   countdown; the certificate near expiry is visually flagged as "expiring soon" (US1).
4. Select the Certificate stuck at a Challenge; confirm the chain view shows
   Certificate → CertificateRequest → Order → Challenge and marks the Challenge as the stuck stage
   with its failure reason/message (US2).
5. Point Fathom at a cluster without cert-manager; confirm the pane shows the honest
   "cert-manager not installed" empty state, not a blank/error view (US3).
6. Point Fathom at a cluster with cert-manager installed but zero Certificates; confirm a distinct
   "no certificates found" state (spec Acceptance Scenario, US3).

## Automated validation

- `cargo test` — protocol round-trip tests (`refresh_cert_manager_roundtrip`,
  `select_certificate_chain_roundtrip`) plus unit tests for expiry classification (R5) and stuck-
  stage classification (R4) in `src/backend/cert_manager.rs`.
- `pnpm -C ui test` — Vitest unit tests for countdown formatting and store ingestion of
  `CertManagerOverviewUpdated`/`CertificateChainUpdated`.
- `pnpm -C ui exec playwright test ui/tests/e2e/cert-manager-pane.spec.ts` — scripted-backend e2e
  covering: expiry countdown rendering, failed-cert chain drill-down showing the stuck stage, and
  the cert-manager-not-installed empty state. See `contracts/protocol.md` for the exact
  command/event shapes the fixture backend must emit.

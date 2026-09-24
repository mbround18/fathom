# Quickstart: Kubernetes Cluster Version Badge

## Prerequisites

- Rust toolchain matching this repo's edition (2024), `cargo`
- `pnpm` with the `ui/` workspace installed (`pnpm -C ui install`)
- A kubeconfig context (a `kind`/`minikube` cluster is sufficient — no special permissions needed
  beyond what Fathom already requires to list nodes)

## Backend validation

1. `cargo build` — confirms `ClusterVersionSummary`, the new `SystemEvent` variant, and the new
   `cluster_version` backend module compile.
2. `cargo test --test protocol_serde` — confirms the round-trip test for
   `ClusterVersionSummary` and the `all_system_events_is_exhaustive` match both pass (the latter
   fails to compile if a variant is ever added without updating this test, per the file's own
   documented convention).
3. Connect Fathom to a real cluster (`cargo run` or the packaged app) and watch backend logs for
   the periodic cluster-version poll (same 20s cadence as the existing Flux/Hubble pollers);
   confirm no errors are logged on a healthy cluster.

## Frontend validation

1. `pnpm -C ui typecheck` and `pnpm -C ui lint` — confirm the new `ClusterVersionBadge` component
   and `system.ts` handler case type-check cleanly against the regenerated `ClusterVersionSummary`
   / `SystemEvent` bindings.
2. `pnpm -C ui test:e2e -- uc-cluster-version-badge` (or the project's equivalent Playwright
   invocation) — runs the new hyper-focused e2e spec against the fake/scripted backend
   (`window.__fathomFake`), covering:
   - The badge renders the server version from an injected `ClusterVersionUpdated` fixture event
     with `skew_detected: false`.
   - The badge shows a skew warning indicator when a `ClusterVersionUpdated` fixture event has
     `skew_detected: true`, and the distinct kubelet versions are inspectable via hover/click.
3. Manual smoke: run the dev app (`pnpm -C ui dev` + `cargo run`, or the project's standard dev
   launch), connect to a cluster, and confirm the badge appears next to
   `FederationContextSwitcher` in the app shell without any visible delay to the rest of the shell
   rendering.

## Expected outcome

- Badge visible in the app shell at all width classes, showing e.g. `v1.29.4`.
- On a cluster with mixed kubelet versions, a warning indicator appears; hovering/clicking reveals
  the distinct versions present.
- Switching cluster context updates the badge to the new cluster's version with no stale flash of
  the previous cluster's data (verified by the existing `WatchTasks::abort_all()` restart
  behavior this feature reuses).

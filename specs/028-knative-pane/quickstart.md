# Quickstart: Validating the Knative Pane

## Prerequisites

- A cluster (or `kind`/`k3d` cluster, or the project's fixture/fake-backend harness used by
  existing Playwright specs) with Knative Serving CRDs installed.
- At least one Knative `Service` with a `Revision` and a `Route` whose `status.traffic` splits
  across two Revisions, to exercise the traffic-split visualization.
- One Revision scaled to zero (idle) to exercise the "why" explanation.

## Backend validation

```bash
cargo build --workspace --all-targets
cargo clippy --all-targets -- -D warnings
cargo test -p fathom knative
```

Expect: `knative::tests::*` unit tests pass (traffic resolution, scale-reason classification),
and the new `refresh_knative_roundtrip` / `KnativeOverviewUpdated` protocol roundtrip tests pass.

An optional, `#[ignore]`d live-cluster test (mirroring `scan_flux_against_live_cluster`) can be
run manually against a real Knative-enabled cluster:

```bash
cargo test -p fathom scan_knative_against_live_cluster -- --ignored --nocapture
```

## Frontend validation

```bash
pnpm -C ui typecheck
pnpm -C ui lint
pnpm -C ui test
```

Expect: new unit tests for `KnativeScaleReason` label formatting and the Route→Revision traffic
graph builder pass; existing topology tests still pass (edge `label` is optional/additive).

## End-to-end validation

```bash
pnpm -C ui test:e2e -- knative-pane
```

Drives the app against a scripted fake backend (per constitution Principle III) that emits a
`KnativeOverviewUpdated` event with a fixture `KnativeOverview`, and asserts:

1. Opening the Knative pane against the fixture renders the Service/Revision/Route lists.
2. The traffic-split visualization shows the correct percentages for a 2-way split.
3. A scaled-to-zero Revision's detail shows the correct "why" text.
4. Against a fixture where `installed: false`, the pane shows the honest "Knative not installed"
   empty state and renders no lists/visualization.

## Manual smoke test

1. `pnpm -C ui dev` (or the project's normal Tauri dev run) against a cluster with Knative
   Serving installed.
2. Open the Knative pane from the tool-pane switcher.
3. Confirm Services/Revisions/Routes populate within ~20s (or immediately, since opening the pane
   sends `RefreshKnative`).
4. Confirm the traffic-split graph renders percentage-labeled edges from a Route to its target
   Revisions.
5. Click a Revision at 0 replicas and confirm its scale-reason explanation is legible and
   non-technical.

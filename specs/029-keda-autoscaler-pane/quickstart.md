# Quickstart: KEDA Autoscaler Pane

## Prerequisites

- Rust toolchain + `cargo` (workspace builds via `cargo build --workspace --all-targets`)
- Node/pnpm for the frontend (`pnpm -C ui install` if not already done)
- Playwright browsers installed for `ui/` e2e (`pnpm -C ui exec playwright install` if needed)
- No live Kubernetes cluster is required to validate this feature — validation is done against
  Fathom's existing scripted-fake-backend e2e harness (`ui/tests/e2e/helpers.ts`,
  `ui/tests/fixtures/builders.ts`), consistent with constitution III.

## Backend validation (unit + protocol)

```sh
cargo test keda            # ScaleReasoning derivation unit tests, keda.rs summarization tests
cargo test --test protocol_serde   # round-trip tests for the new cmd/event variants
```

Expected outcome: all new tests pass; existing tests remain green (no regressions in
`related.rs`'s existing arms or `flux.rs`).

## Frontend validation (unit + typecheck + lint)

```sh
pnpm -C ui typecheck
pnpm -C ui lint
pnpm -C ui test             # unit tests, if any pure-logic helpers are added under ui/src/resources/keda.tsx
```

## End-to-end validation scenario (drives `ui/tests/e2e/keda-autoscaler-pane.spec.ts`)

Mirrors `us1-applications-list.spec.ts`'s hand-built-fixture convention (Applications/KEDA have
no dedicated `cluster()` scenario builder in `tests/fixtures/scenarios.ts`, so fixtures are
emitted directly in the spec via `batch`/`list`/`nsRoute` helpers).

1. Open the app, navigate to the "Autoscalers" section.
2. Assert `RefreshKeda` was sent (`sentTypes(page)` contains it).
3. **KEDA-not-installed case**: emit `KedaOverviewUpdated` with `installed: false`. Assert the
   pane shows the "KEDA not detected" honest-empty-state text, and zero rows.
4. **Happy path**: emit `KedaOverviewUpdated` with `installed: true` and one `ScaledObject`
   fixture, e.g.:
   ```ts
   const QUEUE_WORKER = {
     kind: 'ScaledObject',
     name: 'queue-worker',
     namespace: 'default',
     target_kind: 'Deployment',
     target_name: 'queue-worker',
     target_found: true,
     current_replicas: 4,
     min_replicas: 1,
     max_replicas: 10,
     paused: false,
     ready: true,
     trigger_types: ['prometheus'],
     age: '3d',
   }
   ```
   Assert the row renders with namespace, current replicas (4), and min/max (1/10).
5. Select the row; emit `KedaResourceDetailsUpdated` with a `KedaResourceDetails` fixture whose
   single `prometheus` trigger has `threshold: "50"`, `current_value: 73`, and whose `reasoning`
   is `{ state: "MidRange", message: "..." }`. Assert the Inspector shows the trigger type
   ("Prometheus"), the current/threshold values, and the reasoning message text.
6. Assert min/max-relative reasoning renders distinctly for at least one boundary case (e.g. a
   second fixture with `current_replicas === min_replicas`, asserting the "AtFloor" message
   text), keeping the spec tight per the task's "hyper-focused" instruction — this does not need
   to enumerate every `ScaleReasoningState` variant in e2e (those are covered by the Rust unit
   tests in data-model.md); e2e only needs to prove the wiring renders backend-provided reasoning
   correctly.

Expected outcome: both the "KEDA not installed" and "happy path with correct reasoning" scenarios
pass; test file stays small and focused (per task instruction — not a full trigger-type matrix).

## Manual smoke check (optional, real cluster)

```sh
kubectl apply -f https://github.com/kedacore/keda/releases/latest/download/keda-2.x.x.yaml
kubectl apply -f <a ScaledObject manifest with min=1,max=10, currently mid-scale>
```

Open Fathom against that cluster's context, navigate to Autoscalers, confirm the row and
Inspector match the live `kubectl get scaledobject -o yaml` values.

# Phase 1 Data Model: KEDA Autoscaler Pane

All types below are added to `src/protocol/v1/keda.rs`, following the `TS`-derive/`serde` pattern
already used by `src/protocol/v1/flux.rs`, and exported to the frontend via the existing `ts-rs`
codegen pipeline (no manual TypeScript type authoring).

## KedaOverview

Dashboard-level aggregate, analogous to `FluxOverview`.

| Field | Type | Notes |
|---|---|---|
| `installed` | `bool` | True if `scaledobjects.keda.sh` or `scaledjobs.keda.sh` is a served CRD (research.md R1) |
| `scaled_objects` | `Vec<KedaResourceSummary>` | One per `ScaledObject`, namespace-filtered like every other list |
| `scaled_jobs` | `Vec<KedaResourceSummary>` | One per `ScaledJob` |

## KedaResourceSummary

One row in the Autoscalers list.

| Field | Type | Notes |
|---|---|---|
| `kind` | `String` | `"ScaledObject"` or `"ScaledJob"` |
| `name` | `String` | |
| `namespace` | `String` | |
| `target_kind` | `String` | e.g. `"Deployment"`, `"StatefulSet"`, `"Job"` (from `spec.scaleTargetRef.kind`, defaulting to `Deployment` for ScaledObject per KEDA's own default) |
| `target_name` | `String` | `spec.scaleTargetRef.name` |
| `target_found` | `bool` | False when the target workload no longer exists (FR-014) |
| `current_replicas` | `i32` | From `status.hpaReplicas` or the target workload's live replica count when KEDA hasn't reported it yet |
| `min_replicas` | `i32` | `spec.minReplicaCount`, default `0` |
| `max_replicas` | `i32` | `spec.maxReplicaCount`, default KEDA's built-in default (100) when unset |
| `paused` | `bool` | True when the `autoscaling.keda.sh/paused` annotation is set |
| `ready` | `bool` | From the `Ready`/`Active` conditions in `status.conditions`, when present |
| `trigger_types` | `Vec<String>` | Distinct trigger type names, for the list's compact "trigger" column (full detail lives in `KedaResourceDetails`) |
| `age` | `String` | Formatted like every other resource list's age column |

## KedaResourceDetails

Returned by `SelectScaledObject`/`SelectScaledJob`; superset of `KedaResourceSummary` plus:

| Field | Type | Notes |
|---|---|---|
| `summary` | `KedaResourceSummary` | Embedded, avoids duplicting shared fields |
| `triggers` | `Vec<KedaTrigger>` | Full trigger list with metadata and, when available, live value |
| `reasoning` | `ScaleReasoning` | Derived plain-language explanation (FR-009) |

## KedaTrigger

| Field | Type | Notes |
|---|---|---|
| `trigger_type` | `String` | KEDA's own `type` field verbatim, e.g. `"prometheus"`, `"kafka"`, `"cron"`, `"cpu"` |
| `metadata` | `BTreeMap<String, String>` | `spec.triggers[].metadata` verbatim, generic key/value (FR-008, FR-020 — no per-type parsing beyond what's needed for `current_value`) |
| `auth_ref` | `Option<TriggerAuthRef>` | Present when `spec.triggers[].authenticationRef` is set |
| `active` | `Option<bool>` | From `status.conditions`/`status.externalMetricNames` correlation when KEDA reports per-trigger activity; `None` when not derivable |
| `current_value` | `Option<f64>` | Populated only for `trigger_type == "prometheus"` when Fathom's Prometheus integration is configured and reachable (research.md R3); `None` otherwise — the UI's fallback path (FR-012) |
| `threshold` | `Option<String>` | Convenience copy of `metadata["threshold"]`/`metadata["value"]` (KEDA's field name varies by scaler) for display next to `current_value` |

## TriggerAuthRef

| Field | Type | Notes |
|---|---|---|
| `name` | `String` | |
| `cluster_scoped` | `bool` | True for `ClusterTriggerAuthentication`, false for `TriggerAuthentication` |

*No credential/secret fields exist on this type by design (research.md R6, constitution VI).*

## ScaleReasoning

Derived, non-Kubernetes-native explanation (spec's "Scale Reasoning" key entity).

| Field | Type | Notes |
|---|---|---|
| `state` | `ScaleReasoningState` (enum: `AtFloor`, `AtCeiling`, `MidRange`, `Paused`, `ScaledToZero`, `TargetNotFound`) | Derived by comparing `current_replicas` to `min_replicas`/`max_replicas`, `paused`, and target existence |
| `message` | `String` | Plain-language sentence for the Inspector (e.g. "At floor (1) — no trigger currently reports active demand.") |

Derivation rules (implemented as pure, unit-tested Rust logic in `keda.rs`, not in the frontend,
per constitution II "backend owns the cluster" — the *inputs* are cluster facts, the *reasoning*
over those facts is still backend logic here because it only needs the already-fetched
summary/trigger data, unlike the pod-lifecycle timeline's event-correlation narrative which stays
frontend-side per spec 008's precedent):

1. `target_found == false` → `TargetNotFound`.
2. `paused == true` → `Paused`.
3. `min_replicas == 0 && current_replicas == 0` → `ScaledToZero`.
4. `current_replicas <= min_replicas` → `AtFloor`.
5. `current_replicas >= max_replicas` → `AtCeiling`.
6. otherwise → `MidRange`.

## Relationships (extends `related.rs`'s vocabulary table from spec 004)

| From | Relation | To |
|---|---|---|
| ScaledObject/ScaledJob | `scales` | target workload (Deployment/StatefulSet/Job) |
| ScaledObject/ScaledJob | `owns` (transitive, via target workload's pod selector) | Pod(s) |
| Pod | `scaledBy` | ScaledObject/ScaledJob (reverse of the above, resolved via the pod's owning workload) |

## Protocol additions summary (see contracts/protocol.md for full signatures)

- `BackendCmdV1::RefreshKeda`
- `BackendCmdV1::SelectScaledObject { namespace: String, name: String }`
- `BackendCmdV1::SelectScaledJob { namespace: String, name: String }`
- `SystemEvent::KedaOverviewUpdated(KedaOverview)`
- `SystemEvent::KedaResourceDetailsUpdated(KedaResourceDetails)`
- `SystemEvent::KedaResourceNotFound { kind: String, namespace: String, name: String }`

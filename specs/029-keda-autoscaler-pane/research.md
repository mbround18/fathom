# Phase 0 Research: KEDA Autoscaler Pane

## R1: CRD detection pattern to reuse

**Decision**: Model KEDA detection/summarization as a new bespoke module `src/backend/keda.rs`,
following the `src/backend/flux.rs` pattern — not the generic `src/backend/crds.rs` dynamic-object
browser.

**Rationale**: `flux.rs` discovers served CRDs by listing `CustomResourceDefinition` objects and
matching well-known `plural.group` names (`find_served`), then reads instances generically via
`Api::<DynamicObject>` scoped by the discovered `ApiResource`/`GroupVersionKind`
(`list_instances`). This avoids pinning a KEDA API version (`keda.sh/v1alpha1` today, but the
group/kind names are stable) and matches Fathom's existing convention for "detect an optional
CRD-based integration, then summarize its instances into a purpose-built overview type." The
generic `crds.rs` browser is the right tool for *ad hoc* CRD browsing with no bespoke semantics,
but this feature explicitly needs derived semantics — "why current replicas differ from min/max,"
active/inactive trigger status, target-workload resolution — that the generic browser does not
compute. A bespoke module mirroring `flux.rs` is therefore the better fit, consistent with the
task's instruction to reuse the Flux "detect-CRD-and-show" pattern.

**Alternatives considered**:
- Generic `crds.rs` dynamic-object browsing with client-side interpretation only: rejected because
  the min/max/trigger-reasoning logic belongs in the backend per the constitution ("backend owns
  the cluster"; UI derives narrative only from backend-provided facts, not from raw spec/status
  parsing of arbitrary dynamic JSON).

**CRD names to detect** (mirroring `flux.rs`'s `SOURCE_CRDS`/`KUSTOMIZATION_CRD` constants):
`scaledobjects.keda.sh`, `scaledjobs.keda.sh`, `triggerauthentications.keda.sh`,
`clustertriggerauthentications.keda.sh`. KEDA is considered "installed" if at least
`scaledobjects.keda.sh` OR `scaledjobs.keda.sh` is served (mirrors Flux's "any bucket present ⇒
installed" rule); Trigger(Cluster)Authentication CRDs are detected for the "referenced by name"
display (FR-010) but do not by themselves imply KEDA is "installed" for pane-visibility purposes.

## R2: Watch/event wiring pattern to reuse

**Decision**: Follow the `poll_flux_overview` background-poll + `RefreshKeda` on-demand command
pattern, emitting a single dashboard-level `SystemEvent::KedaOverviewUpdated(KedaOverview)` for the
list pane, plus a `SelectScaledObject`/`SelectScaledJob` command (mirroring `SelectFluxResource`)
for the Inspector's single-resource detail (including trigger reasoning and, when available, live
metric values), emitting `KedaResourceDetailsUpdated` / `KedaResourceNotFound`.

**Rationale**: Matches an already-shipped, well-understood pattern (`src/backend/sync.rs`'s
`ensure_flux_poll`, `src/protocol/v1/cmd.rs`'s `RefreshFlux`/`SelectFluxResource`,
`src/protocol/v1/event.rs`'s `FluxOverviewUpdated`/`FluxResourceDetailsUpdated`/
`FluxResourceNotFound`). Reusing this shape keeps the frontend integration (Zustand slice +
handler + panel component) structurally identical to `FluxPanel.tsx`, minimizing new patterns for
reviewers and for the e2e test harness (which already knows how to script Flux-shaped fixture
events).

**Alternatives considered**: A dedicated watch-stream (`kube::runtime::watcher`) per ScaledObject
was considered for lower latency, but rejected for v1 — the existing 20s dashboard poll cadence
matches Flux's UX expectations and avoids a new streaming-lifecycle pattern; can be revisited later
without a protocol break (polling command names stay the same).

## R3: Metrics/Prometheus reuse for live trigger values

**Decision**: Reuse `PrometheusClient::query_instant` (in `src/backend/metrics_provider.rs`,
already generic — accepts an arbitrary PromQL string) **only** for triggers of type `prometheus`,
building the query directly from that trigger's own `metadata.serverAddress` +
`metadata.query`/`metadata.metricName` + `metadata.threshold` fields (already present verbatim in
the `ScaledObject`/`ScaledJob` spec — no new query-building heuristics needed). Add one narrow
backend command/path (e.g. folded into `SelectScaledObject`'s response, not a separate command) that
calls `query_instant` against the trigger's own `serverAddress` when Fathom's metrics integration
considers Prometheus reachable, and populates a `current_value: Option<f64>` field on that
trigger's protocol type. Every other trigger type (kafka, redis, cron, cpu, memory, external,
etc.) renders trigger-type + full metadata key/value display only, per the spec's explicit
non-hard-dependency and non-goal (no custom trigger-type-specific UI/plumbing).

**Rationale**: Confirmed via codebase research that `query_instant` exists and is generic, but is
currently `#[allow(dead_code)]`/unwired — no existing `BackendCmd` exposes arbitrary-PromQL
querying to the frontend, and every real call site in `src/backend/metrics.rs` builds fixed
CPU/memory PromQL strings for pod/container metrics, not arbitrary queries. Wiring a fully generic
"query any PromQL from the frontend" command is unnecessary and out of scope; the Prometheus
trigger case is the only one where Fathom can construct a *correct* query without any new
domain knowledge, because the query text is not synthesized — it is quoted verbatim from the
already-present trigger metadata written by whoever authored the `ScaledObject`. This satisfies the
task's instruction to fall back to trigger-type + threshold display when live-metric reuse is not
"readily reusable" for other trigger types.

**Alternatives considered**:
- Building a generic external-scaler / metrics-API abstraction that queries KEDA's own
  `external.metrics.k8s.io` adapter API (what KEDA itself uses) for *any* trigger type: rejected as
  substantial new plumbing (a new Kubernetes aggregated-API client, no existing Fathom precedent)
  that the task explicitly says to avoid building for this spec.
- Reusing `query_instant` for the `cpu`/`memory` KEDA trigger types too (since KEDA's built-in
  cpu/memory scalers read from the metrics API, not Prometheus): rejected for v1 — those triggers
  don't carry a `serverAddress`, so there is no reachable Prometheus endpoint to query without
  guessing Fathom's own configured metrics stack's target; deferred, and the trigger still renders
  correctly via the threshold-only fallback.

## R4: Generic related-resource interlinking reuse

**Decision**: Add two new `match kind` arms to `src/backend/related.rs::related_resources`:
`"ScaledObject" | "ScaledJob"` → resolve the target workload (Deployment/StatefulSet for
ScaledObject, Job for ScaledJob) and, transitively, its currently running Pods (reusing
`related_for_workload`'s pod-selector logic); and extend `"Pod"`'s existing arm
(`related_for_pod`) to additionally look up any `ScaledObject`/`ScaledJob` whose
`spec.scaleTargetRef` resolves to the pod's owning workload, surfacing a `scaledBy` relationship
back to it.

**Rationale**: This is exactly the extension point `related.rs`'s module doc comment describes
("one dispatch point, keyed by kind, used by every inspector that has a defined relationship").
No frontend-side generic resolver exists or is needed — the UI already renders whatever
`RelatedResourceRef` list a `RefreshRelatedResources` response contains, generically, for any kind.

**Alternatives considered**: A `ServiceAccount`-style "left for a future spec" punt was considered
for the Pod→ScaledObject direction, but rejected — `scaleTargetRef` resolution from a Pod's owning
workload back to a `ScaledObject` in the same namespace is O(number of ScaledObjects in the
namespace), cheap, and the spec (FR-016) requires it directly, unlike the harder CRD-instance
disambiguation problem `related.rs` already documents as deferred.

## R5: Pod-lifecycle timeline linkage

**Decision**: Treat the "pod-lifecycle timeline" linkage (User Story 4, FR-017) as a thin
reference, not new scale-history plumbing: the Pod Inspector's existing related-resources section
(already rendered generically per R4) surfaces the link to the responsible `ScaledObject`/
`ScaledJob`; the pod-lifecycle diagnostics view (`ui/src/resources/podLifecycle.ts`, spec 008)
requires no new backend event stream, since KEDA does not natively expose a "replica change
history" resource distinct from what the target workload's own ReplicaSet/HPA-equivalent already
does. No new `HorizontalPodAutoscaler`-events integration is introduced in this spec — it is
noted as a natural but explicitly out-of-scope follow-up in Assumptions/plan.

**Rationale**: Matches the task's guidance to link "where reasonable" rather than rebuild the
pod-lifecycle data model, and keeps this spec's backend surface limited to KEDA's own CRDs.

## R6: Secrets safety (TriggerAuthentication)

**Decision**: `TriggerAuthentication`/`ClusterTriggerAuthentication` are surfaced by name/scope
only (`referenced_auth: Option<TriggerAuthRef>` on a trigger's protocol type); Fathom never
resolves or displays the `Secret`/`env`/`configMap` values they point to.

**Rationale**: Constitution VI ("Secrets and credentials MUST NOT be ... shown in the UI unless the
user explicitly reveals them") and the spec's own edge case ("must not leak credential material").
No existing Fathom code path resolves TriggerAuthentication secret refs, so nothing needs to be
disabled — this is a "don't build it" decision, not a redaction of existing behavior.

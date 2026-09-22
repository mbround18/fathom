# Research: Applications View & Rich Interlinking

## R1: How to attribute a live resource to a Helm release

**Decision**: Use the annotations Helm 3 sets automatically on every object it applies —
`meta.helm.sh/release-name` and `meta.helm.sh/release-namespace` — plus the
`app.kubernetes.io/managed-by: Helm` label as a secondary confirmation signal. Group by
`(release-namespace, release-name)`.

**Rationale**: This is the actual mechanism Helm 3 uses for its own object tracking (it has no
owner-reference chain of its own outside StatefulSet-style built-ins); it requires no read of
Helm's release-storage Secrets/ConfigMaps, matching FR-019/the spec's explicit non-goal. The
feature description referred to these generically as "labels" — the annotation is the reliable
identity signal, the label is present on most but not strictly all chart-templated objects, so the
annotation is primary and the label is used only to raise confidence / disambiguate the
"label-only heuristic" tier in FR-004's Managed By column.

**Alternatives considered**: Reading Helm's `helm.sh/release.v1` Secrets directly (rejected: that
*is* "Helm release reading/parsing," explicitly out of scope per FR-019 and reserved for the
future §5a spec) — kept as a forward pointer for that later feature, not built here.

## R2: How to attribute a live resource to a Flux-managed application

**Decision**: Two different mechanisms for Flux's two managed-application kinds, since Flux
tracks ownership differently for each:

- **Kustomization**: read `status.inventory.entries` on the `Kustomization` object itself — a
  list of `{ id: "<namespace>_<name>_<group>_<kind>", v: "<version>" }` strings Flux already
  writes there for its own prune/GC bookkeeping. This is a new field read on an object Fathom's
  Flux integration already fetches (`src/backend/flux.rs` already lists `Kustomization` objects
  for `FluxOverview`); it is not a new external call or new watch.
- **HelmRelease**: a Flux `HelmRelease` does not carry its own resource inventory — the
  underlying Helm release it drives is tracked exactly like a normal Helm release (release name
  derived from `<HelmRelease name>` in the `HelmRelease`'s own namespace, or
  `spec.releaseName`/`spec.targetNamespace` when set). So Flux-managed Helm applications are
  attributed via the **same** annotation mechanism as R1, keyed off the `HelmRelease`'s resolved
  release name/namespace, not a separate Flux-specific mechanism.

**Rationale**: This corrects an assumption in the original feature ask ("Flux tracks exactly which
resources each Kustomization/HelmRelease owns" as if uniformly true) — it's true for
Kustomizations via `status.inventory`, but a Flux `HelmRelease` is Helm underneath, so its
inventory comes from Helm's own annotation convention. Getting this right in research now avoids
a wrong data-model assumption at implementation time.

**Alternatives considered**: Parsing the Helm storage Secret that a Flux `HelmRelease` drives to
get its manifest-derived resource list directly (rejected — same as R1, this is real Helm-release
reading, out of scope).

## R3: Precedence when both Helm and Flux signals are present on the same resource

**Decision**: If a resource's `meta.helm.sh/release-name` matches a release Flux is actively
driving via a `HelmRelease` (i.e. the release name/namespace resolves to a known `HelmRelease`),
attribute the resource to the Flux `HelmRelease` application, not a separate "Helm" application
with the same underlying release. If a resource's owner-reference/inventory match points to a
`Kustomization`, attribute it there. A resource with only the Helm annotations and no
corresponding Flux object is attributed to a plain "Helm" application.

**Rationale**: Directly satisfies FR-003 (no double-listing) and matches the spec's edge case
("Flux takes precedence, since Flux is the actively reconciling authority").

## R4: Application list refresh mechanism — no new watch/poll loop

**Decision**: Applications are computed on demand, triggered by a new `RefreshApplications`
command (and once automatically after `SetContext`/`RefreshNamespaces`, mirroring how
`RefreshCrds`/Flux's own refresh already behave), not by a new continuously-running watch or
timer-driven poll task.

**Rationale**: Surveying `src/backend/state.rs`'s `WatchTasks`, only Pods and Deployments have a
live `kube::runtime` watch; every other resource kind (including Flux's own `FluxOverviewUpdated`)
is refreshed on an explicit `RefreshX` command, listing fresh data synchronously. Applications is
a pure synthesis over data other `RefreshX`/`SelectX` handlers already fetch (Pods, Deployments,
StatefulSets, ReplicaSets, Services, the Flux `Kustomization`/`HelmRelease` lists) — it does not
need its own independent freshness cadence beyond "recompute when asked," consistent with the
majority pattern in this codebase and avoiding an unjustified new background task per the "only
add a new poll loop if nothing existing already covers it" guidance.

**Alternatives considered**: A dedicated interval-poll task like Hubble's (rejected — Hubble polls
because it hits an external gRPC service with real network cost worth rate-limiting; this feature
reads already-in-process Kubernetes API data with no comparable cost, so on-demand refresh is
simpler and sufficient).

## R5: Related-resources resolver — data source and call shape

**Decision**: One backend function,
`related_resources(kind: &str, namespace: Option<&str>, name: &str, owner_refs: &[OwnerReference], labels: &BTreeMap<String,String>) -> Vec<RelatedResourceRef>`,
callable from any existing `SelectX` read handler. It is populated from two sources, in priority
order:
1. **Data already being returned for the current resource's own read**: e.g. a Pod's own
   `ownerReferences` (already present on the typed object Fathom already fetches for
   `PodDetailsUpdated`) resolves its owning ReplicaSet/Application without any extra API call.
2. **A targeted lookup** only when the relationship isn't derivable from data already in hand
   (e.g. "which Pods does this Service select" requires listing Pods with the Service's selector
   as a field/label selector — the same `Api<Pod>::list` with a `ListParams::labels(..)` pattern
   already used elsewhere in the backend for filtered lists, not a full unfiltered list-then-filter
   scan).

**Rationale**: Directly satisfies FR-009 (generic, kind-keyed resolver) and the constraint to
prefer already-fetched data over a redundant round trip, mirroring the "native kube-rs API, no
reimplemented logic" principle `003`'s plan already established for `evict()`.

**Alternatives considered**: A separate one-off relationship function per resource kind (rejected
— this is exactly the per-kind duplication FR-009 exists to avoid, and is the status quo today,
per `workloads.tsx`'s existing non-clickable `pods` list, which already re-derives pod membership
per kind rather than through any shared mechanism).

## R6: Frontend drill-down / back-stack placement

**Decision**: Extend `PaneState` (an Inspector pane, `ui/src/store/index.ts`) with a
`history: ObjectRef[]` array. `openInspector(ref, from?)` gains a `push: boolean` behavior:
selecting a resource from a list pane replaces `target` and clears `history` (a fresh drill-down
root); following a related-resource link from within an inspector pushes the current `target` onto
`history` before replacing it. A new `goBack(paneId)` store action pops `history` and restores it
as `target`. The Inspector renders a breadcrumb strip from `history + [target]` when non-empty.

**Rationale**: `ObjectRef` (`{ section, kind, namespace, name }`) already fully identifies
anything the Inspector can show (`ui/src/resources/types.ts`), and `PaneState` already tracks a
single `target`/`source` pair (`ui/src/store/index.ts`) — this is additive state on an existing
mechanism, not a new router or parallel navigation system, per the plan's constraint to reuse
existing selection state.

**Alternatives considered**: A URL-based router with history (rejected — Fathom's Inspector is
explicitly not route-based today per Principle-aligned prior features; introducing routing here
would be new architecture the plan instructions explicitly ask to avoid).

## R7: Dependencies

**Decision**: No new Rust crate and no new frontend package are required.

**Verification**: `Cargo.toml` already depends on `kube` 2.0.1 with `runtime`/`derive`/`ws`
features (sufficient for `Api<T>::list`/`get`, `ListParams`, and reading `ownerReferences` off any
typed object) and `tokio` 1.49 (full). `ui/package.json` already has `zustand` 5 for store state
and `react` 19; a breadcrumb/back-stack is plain application state (R6), not a routing library.

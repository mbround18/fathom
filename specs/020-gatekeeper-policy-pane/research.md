# Phase 0 Research: Gatekeeper Policy Pane

All unknowns below were resolved by reading the existing Flux panel implementation
(`src/backend/flux.rs`, `src/protocol/v1/flux.rs`, `src/backend/sync.rs`, `ui/src/components/panels/FluxPanel.tsx`)
and the Hubble panel and Applications-interlinking (`RelatedResourceRef`) code as the established
precedent for every open question. No technology choice in this feature departs from what those
features already do.

## R1: How to detect an operator/CRD without a hard dependency on Gatekeeper being installed

**Decision**: List `CustomResourceDefinition` objects cluster-wide via `Api<CustomResourceDefinition>::all(client).list(...)`
and check for `constrainttemplates.templates.gatekeeper.sh` (and, to enumerate constraint kinds,
CRDs whose group is `constraints.gatekeeper.sh`). This is byte-for-byte the same discovery
mechanism `flux.rs::find_served` uses for Flux's CRDs.

**Rationale**: Requires no new permission model, no new dependency, and matches the CRD-presence
detection Fathom already performs for Flux and Longhorn. `installed: false` (empty lists) is
returned rather than an error when no matching CRDs exist, exactly like `scan_flux`.

**Alternatives considered**:
- Probing a well-known Gatekeeper `Deployment`/`Service` name (like Hubble's `hubble-relay`
  Service probe) — rejected because Gatekeeper's constraint enforcement is entirely CRD-driven;
  the CRDs are the authoritative signal, and the constraint *data* we need to display only exists
  via the CRD API anyway, so there is no reason to add a second detection signal.

## R2: How to enumerate constraints when each constraint kind is its own CRD

**Decision**: List all CRDs in the `constraints.gatekeeper.sh` API group (each one is a distinct
constraint *kind*, e.g. `K8sRequiredLabels`, `K8sDisallowedTags` — Gatekeeper dynamically registers
one CRD per `ConstraintTemplate`). For each such CRD, use the same `ApiResource::from_gvk` +
`Api<DynamicObject>::all_with` pattern `flux.rs::list_instances` already uses, then read each
instance's `.status`.

**Rationale**: This is the exact multi-CRD "one logical concept, several concrete kinds" shape
Flux already has (GitRepository/OCIRepository/HelmRepository/Bucket all being "sources"), just
discovered by group filter instead of a fixed name list — no new discovery pattern is invented,
only the CRD-selection predicate changes (by API group prefix rather than an exact name list).

**Alternatives considered**:
- Requiring the operator to know all constraint kinds in advance and adding a fixed name list —
  rejected: constraint kinds are arbitrary, user-defined-by-template names; a fixed list would
  silently miss policies added after this feature ships.

## R3: Where violation counts/detail come from and how to handle Gatekeeper's truncation

**Decision**: Read `.status.totalViolations` for the count (falling back to `.status.violations`
array length if that field is absent), and `.status.violations[]` (each with `enforcementAction`,
`kind`, `name`, `namespace`, `message`) for the drill-down list. When `totalViolations` exceeds
`violations.len()`, surface both numbers so the UI can render "showing N of M" (FR-007) — this
mirrors Gatekeeper's own documented behavior: `.status.violations` is capped by the audit
controller's `--constraint-violations-limit` flag (default 20), while `totalViolations` reports
the true count.

**Rationale**: These are the only fields Gatekeeper's audit controller populates; no separate API
call or CRD is needed to get violation detail, keeping this a single-CRD-family read exactly like
`Constraint`'s own status is the source of truth (no analogue is needed to Flux's
`status.artifact`/`status.lastAppliedRevision` split, since Constraint is simpler).

**Alternatives considered**:
- Fetching the actual violating object live via a follow-up `Api::get` per violation to enrich
  the display — rejected for this feature: the violation's own `namespace`/`kind`/`name`/`message`
  fields are sufficient to render the drill-down list and to build the Inspector `ObjectRef`; a
  live fetch is unnecessary and the existing Inspector, once opened, already re-fetches current
  state for that object (also directly satisfies edge case: a since-deleted violating resource
  shows the Inspector's own not-found state, not a stale duplicate here).

## R4: How to resolve a violating resource's Inspector navigation target (section + kind)

**Decision**: Reuse the same `kind → SectionKey` mapping pattern `applications.rs::workload_owns_ref`
already uses (`match kind { "Deployment" => section_keys::workloads_deployments(), "Pod" =>
section_keys::workloads_pods(), ... }`), extended to cover the built-in kinds Gatekeeper commonly
targets, with `section_keys::custom_resources()` as the fallback for any kind not in that map
(covers CRD-instance violations, e.g. a policy that constrains a Flux `Kustomization` or another
CRD). The resulting `SectionKey` is carried on each violation entry (mirroring
`RelatedResourceRef.section`, which does exactly this for the Applications-interlinking feature),
so the frontend's `openInspector(ref)` call needs no kind-mapping logic of its own.

**Rationale**: This is a straight reuse of an established, already-tested mapping approach
(`RelatedResourceRef`) rather than inventing a new resolution mechanism, and keeps the "backend
computes navigation targets, frontend just calls `openInspector`" boundary from Principle II
intact.

**Alternatives considered**:
- Sending only `kind`/`namespace`/`name` and having the frontend guess the section from a
  hardcoded map — rejected: this duplicates backend logic in the frontend and drifts every time a
  new resource kind is added; `RelatedResourceRef` already proves the backend-side approach works.

## R5: Live-update mechanism (watch vs. poll)

**Decision**: A 20-second background poll task (`poll_gatekeeper_overview`, spawned once by
`ensure_gatekeeper_poll`, guarded by `state.watch_tasks.gatekeeper_poll_task.is_some()`), emitting
`SystemEvent::GatekeeperOverviewUpdated` — identical cadence and task-lifecycle pattern to
`poll_flux_overview`/`ensure_flux_poll`.

**Rationale**: The spec's Assumptions section explicitly allows "a bounded poll/reconcile fallback
consistent with the Flux panel's own implementation," and Flux itself uses a 20s poll rather than
a raw `kube::runtime::watcher` stream (unlike the Pod/Deployment watchers in the same file), so
this is not a lesser pattern — it *is* the established pattern for CRD-status dashboards in this
codebase (Hubble uses the identical cadence for its own reasons). Reusing it keeps identical
behavior and testing story to the two panels this feature is explicitly modeled on.

**Alternatives considered**:
- A `kube::runtime::watcher` stream on `Constraint` objects — considered but rejected to match
  the plan instruction to mirror Flux's approach exactly; watching would also require per-CRD-kind
  watch streams (since each constraint kind is its own CRD/API resource), adding real complexity
  for a status field that only needs eventual, human-timescale freshness.

## R6: Frontend drill-down interaction (constraint list → violating resources → Inspector)

**Decision**: A new `GatekeeperPanel.tsx`, structured like `FluxPanel.tsx` for its top-level
tiles/empty-states (via `IntegrationSummary`'s tiles), but with its own interactive list body
(not `IntegrationSummary`'s static `rows`) for the two-level drill-down: a clickable constraint
list (local `useState<string | null>` for the selected constraint id, exactly the kind of
ephemeral pane-local selection state already used elsewhere, e.g. `TopologyCanvas`'s hover/selection
state) and, once a constraint is selected, a list of its violations, each rendered as a `<button>`
calling `openInspector(ref, paneId, true)` — the same call `GitOpsWarningBanner.tsx` already makes
for its single link-out.

**Rationale**: `IntegrationSummary` was designed for flat, non-interactive stat panels (Flux,
Hubble) and deliberately has no click/selection support; rather than bolt drill-down interaction
onto a shared component two other shipped features depend on, this feature adds its own small
list UI built from the same primitives (`openInspector`, tone-colored rows) — consistent styling,
no shared-component risk.

**Alternatives considered**:
- Extending `IntegrationSummary` itself with an `onRowClick` prop — rejected: it would need to
  support two nested list levels (constraints, then violations) to serve this feature, which is
  more surface area than Flux/Hubble need; keeping `IntegrationSummary` flat and giving this panel
  its own list avoids scope creep into a component two shipped features already rely on.

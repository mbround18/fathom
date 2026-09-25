# Research: Cluster Sanitizer / Linter

## R1: Rule engine placement — backend vs. frontend

**Decision**: Frontend-only, pure-TS module (`ui/src/linter/`), mirroring spec 007
(topology-graph-view) exactly. No new Rust module, no new protocol command/event for rule
evaluation itself.

**Rationale**: Every resource kind the eight rules need (Pods, Deployments, StatefulSets,
DaemonSets, ReplicaSets, Jobs, CronJobs, ConfigMaps, Secrets, Roles, ClusterRoles, RoleBindings,
ClusterRoleBindings, PodDisruptionBudgets — see R2 for the one exception, ServiceAccounts) already
has a backend `RefreshX`/`SelectX` command and a `ResourceListEvent` variant in
`src/protocol/v1/`. Per Constitution Principle II ("One Contract, Backend Owns the Cluster"), the
backend already owns every cluster read the rules depend on; a derived, read-only *view* over
data the frontend's Zustand store already caches is explicitly the pattern the constitution and
spec 007's plan sanction ("frontend derives a client-side view over already-fetched data"). Adding
a backend "lint" aggregate command would duplicate logic that's naturally a pure function of
already-typed row arrays, and would need its own refresh/staleness story that the store's existing
live-update mechanism (Tauri `Channel`-pushed event batches into `store.lists`) already provides
for free.

**Alternatives considered**:
- A new backend `RunLinter`/`RefreshLintReport` command that re-lists everything server-side and
  returns findings: rejected. It would re-fetch data the frontend may already have cached,
  duplicate the "one contract" resolver pattern for no correctness benefit (the rules are pure
  functions of already-typed structs), and reintroduce a staleness/refresh-cadence problem the
  store's existing watch-driven `lists` cache already solves.
- Splitting rules between backend (e.g., RBAC wildcard detection, since RBAC data is
  security-sensitive) and frontend (everything else): rejected for v1 — unnecessary complexity
  for a fixed, non-authored rule set with no differing trust boundary (all inputs are already
  visible to the operator in existing list panes).

## R2: Resource kinds not yet wired into the frontend store

**Finding**: Six of the fourteen resource kinds needed by the rule set — ConfigMaps, Secrets,
PodDisruptionBudgets, Jobs, CronJobs, and ReplicaSets — have full backend + protocol v1 support
(`src/backend/{configmaps,secrets,poddisruptionbudgets,jobs,cronjobs,replicasets}.rs`,
matching `src/protocol/v1/*.rs` files, `RefreshX` command variants in `cmd.rs`, and
`ResourceListEvent` variants in `event.rs`, with generated TS summary types already present in
`ui/src/protocol/generated/`) but **no frontend descriptor or nav-visible pane has ever wired
them up** (`ui/src/resources/nav.ts` lists them under a comment noting they use "egui labels
until their descriptors land"; they are absent from `ALL_DESCRIPTORS` in
`ui/src/resources/index.ts`). Nothing in the frontend currently dispatches their `RefreshX`
commands, so `store.lists` never contains rows for these six sections today.

**Decision**: Add these six sections to the existing `SECTION` const and `REFRESH` map in
`ui/src/protocol/commands.ts` (the same generic fallback table already used by
`refreshSection()` for any section with no full `ResourceDescriptor` registered — see
`ui/src/store/index.ts`'s `refreshSection`: `d ? d.refresh(route) : refreshCmd(section)`). This
requires zero backend code changes and zero new protocol surface: it is wiring for six
already-shipped, already-typed, currently-unused backend commands, not new cluster-read
capability. The `LinterPanel` triggers these refreshes on mount (exactly like `TopologyPanel`
already does for its own section list), and reads the resulting rows straight out of
`store.lists` by section key — no `ResourceTable`/nav UI is added for these kinds as part of this
feature (that remains separately scoped future work per the T068+ note in `nav.ts`).

**Rationale**: This keeps the "zero new cluster reads" constraint honest in its strongest sense —
no new Rust code, no new protocol command, no new watch — while still letting the Linter
correctly evaluate all rules that don't depend on ServiceAccounts (R3). It reuses the exact
generic mechanism (`refreshCmd` fallback) the codebase already built for sections without a full
descriptor, rather than inventing a parallel fetch path.

**Alternatives considered**: Building full `ResourceDescriptor`s (list columns, detail sections,
actions, nav entries) for all six kinds so they appear as first-class list panes: rejected as
out of scope for this feature — it's a substantial, separately valuable feature (finishing the
T068+ resource-kind port) that a read-only lint pane does not need in order to read their rows out
of the store.

## R3: ServiceAccounts — a genuine gap, not a wiring gap

**Finding**: Unlike the six kinds in R2, `ServiceAccount` has **no existing capability anywhere**
in the codebase: no `src/backend/serviceaccounts.rs`, no protocol v1 file, no `RefreshX`/`SelectX`
command, no generated TS type. `src/backend/related.rs` explicitly notes "`ServiceAccount` is not
yet a resource kind" it can resolve. Fetching ServiceAccounts would be a genuinely new backend
capability, which the feature's "zero new cluster reads" constraint (FR-002) rules out for v1.

**Decision**: The "unused ServiceAccounts" rule (part of the fixed FR-004 rule set) ships as a
rule that always reports itself as **skipped** via the exact mechanism spec.md's FR-010 and the
first Edge Case already require for any rule whose dependent resource kind isn't in the cache:
"the affected rule(s) MUST be skipped for that data... the pane MUST indicate that some checks
were skipped due to limited visibility." The rule module exists (`ui/src/linter/rules/
unusedServiceAccount.ts`), is unit-tested for its skip behavior, and is wired into `evaluate.ts`
identically to any other rule — it simply always finds its required section (`ServiceAccount`
rows) absent and reports the skip, the same code path a real RBAC-visibility gap would hit for
any other rule. No backend work is added to make this rule "real" in v1.

**Rationale**: This resolves the conflict between "ship the fixed 8-rule set named in the spec"
and "zero new cluster reads" without silently dropping a named rule or silently under-reporting.
It is a direct, spec-sanctioned use of the graceful-degradation contract the spec already
requires for exactly this situation. Adding ServiceAccount fetch/watch support is flagged as
future work (a natural follow-up once ServiceAccounts are ported the same way DacronJobs/Jobs
etc. are being wired in R2).

**Alternatives considered**: Dropping the ServiceAccount rule from scope entirely: rejected —
it's explicitly named in the approved feature description, and the "always skipped" behavior
costs nothing extra to implement (it reuses the mandatory skip-reporting mechanism) while leaving
a clean seam for a future spec to light it up for real once ServiceAccount data exists.

## R4: Deprecated API version detection

**Decision**: A small, static, ships-with-the-app table
(`ui/src/linter/deprecatedApiTable.ts`) of `{ apiVersion, kind, removedInVersion?, replacement }`
entries drawn from upstream Kubernetes deprecation guides, checked against each cached resource's
already-known `apiVersion`/`kind` (list rows carry enough identity for this; where a row type
doesn't already surface `apiVersion`, the rule uses `kind` alone against entries that apply
regardless of cluster version, and treats version-gated entries as informational unless the
already-fetched server version, where available, confirms removal).

**Rationale**: Matches spec.md's own Assumptions section (static, ships-with-the-app mapping, not
a live discovery-document query beyond what's already fetched). No backend change needed.

**Alternatives considered**: Querying the cluster's live OpenAPI/discovery document per resource:
rejected — a new cluster read, and overkill for a fixed, infrequently-changing deprecation list.

## R5: Click-through to Inspector

**Decision**: Reuse `useStore().openInspector(ref: ObjectRef, from?: PaneId, push?: boolean)`
unchanged, exactly as `TopologyCanvas.tsx` already does for node clicks and
`ui/src/resources/workloads.tsx`'s `PodLink` does for pod drill-through. Each `Finding` carries
an `ObjectRef` (`{ section, kind, namespace, name }`) built the same way descriptors build refs
today.

**Rationale**: This is precisely the mechanism the spec's User Story 2 asks for, already proven
across three existing call sites; no new store action or protocol surface needed.

## R6: Most rules need per-object Detail data, not just list Summaries

**Finding**: Checking the actual generated protocol types, list-level Summary shapes are
deliberately lightweight and omit exactly the fields the rules need:
`PodSummary` has no resource requests/limits, probes, or owner info; `RoleSummary`/
`ClusterRoleSummary` have only a `rules_count: number`, not the actual verbs/resources;
`WorkloadSummary` has no pod-template label selector. That data exists only on the corresponding
`*Details` type (`PodDetails.resource_specs`/`containers_lifecycle`/`owner`,
`RoleDetails.rules`/`ClusterRoleDetails.rules`, `WorkloadDetails.pod_selector`), which is fetched
per-object via the existing `SelectPod`/`SelectRole`/`SelectClusterRole`/`SelectDeployment`/etc.
commands — today triggered only when an operator opens that specific object in the Inspector.

**Decision**: The Linter triggers these same existing per-object `SelectX` commands automatically,
for every object currently in the active list scope, for the kinds whose rules need detail data
(Pods, Roles, ClusterRoles, and the three workload kinds for their `pod_selector`). This is the
one place the feature causes more reads to happen than if the pane were never opened — but it
adds no new command, no new protocol shape, and no new backend code; it is the existing
"click an object to see its detail" capability, invoked automatically instead of manually. To
respect Principle V (100ms input budget, <2% idle CPU, no runaway fan-out), this fetch is:
- Scoped to the currently active namespace (or the small set of cluster-scoped kinds — Roles are
  namespaced, ClusterRoles are not), never silently expanded to "all namespaces" without the
  operator having actually selected that scope (mirrors FR-011).
- Throttled to a bounded concurrency (a small fixed in-flight limit, e.g. 6–8 concurrent
  `SelectX` sends), so opening the pane against a large namespace doesn't burst hundreds of
  simultaneous requests at once.
- Progressive: `evaluate.ts` recomputes findings incrementally as each detail reply lands in
  `store.details` (the store already updates reactively per reply), so the pane fills in rather
  than blocking on every object's fetch before showing anything (spec.md SC-001, revised).

**Rationale**: There is no way to build the resource-limits, probes, naked-pod, RBAC-wildcard, or
missing-PDB rules faithfully from list Summaries alone — the data simply isn't there. Refusing to
fetch details would mean either not building those rules (unacceptable — they're the spec's
headline examples) or only linting objects the operator happened to have already opened
(defeats the purpose: SC-004 requires finding 5+ known-bad objects "without consulting any other
tool"). Reusing the existing per-object commands, scoped and throttled, is the smallest
change consistent with "no new backend capability."

**Alternatives considered**: A new bulk backend command that returns detail-shaped data for every
object of a kind in one round trip (e.g. `RefreshPodsWithDetails`): would reduce request count and
match "fewer, new reads" more efficiently, but is explicitly ruled out by FR-002's "no new backend
command" constraint and is a natural, separately-scoped future optimization once this feature
proves the value of the checks — flagged as follow-up work, not v1 scope.

## R7: Deprecated API version — what's actually observable

**Finding**: The Kubernetes API server always serves a live object converted to whichever
apiVersion the client requested (kube-rs's typed clients request the current stable version), so
neither a typed `Get`/`List` response nor its raw JSON echo can reveal "this object was originally
created via an old, now-removed apiVersion alias" directly — that information is not part of the
live, convertible representation. The standard mechanism live-cluster tools (e.g. Pluto, kubent)
use instead is `metadata.managedFields[].apiVersion`, which Kubernetes populates per field-manager
write and does **not** get converted away — it's a durable historical record of which apiVersion
each writer (e.g. `kubectl-client-side-apply`, a controller, `helm`) actually used.

**Decision**: The deprecated-API rule reads `metadata.managedFields` out of the object's `raw_json`
(already part of the same `*Details` fetch R6 already triggers for that object) and checks each
entry's `apiVersion` against the static deprecation table (R4), rather than checking the object's
top-level (always-current) `apiVersion`.

**Rationale**: This is the only technically sound way to implement this rule against a live
cluster without a new capability, and it's the same approach established deprecated-version
scanning tools use.

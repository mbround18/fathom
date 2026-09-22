# Phase 0 Research: Resource Editing, Node Actions & Custom Resource Browsing

## R1 — YAML edit + apply mechanism

**Decision**: The frontend edits raw YAML text only (no structural/schema-aware editing). On
"Apply", the text is sent verbatim to the backend. The backend parses it with `serde_yaml` into
`serde_json::Value`, validates `apiVersion`/`kind`/`metadata.namespace`/`metadata.name` are
unchanged from the object that was opened (spec FR-005), then calls `Api::replace()` (typed
resources) or `Api::<DynamicObject>::replace()` (CRD instances) using the `resourceVersion`
carried in the edited YAML. Kubernetes' own optimistic-concurrency check rejects a stale
`resourceVersion` with 409, which is surfaced to the operator as a specific conflict error (FR-004)
with the edit preserved for retry (FR-007) — no bespoke locking or diffing logic needed for
conflict detection itself.

**Rationale**: `kube-rs` 2.0.1's `Api<K>::replace(&self, name, pp, data)` (confirmed at
`kube-client-2.0.1/src/api/core_methods.rs:488`) is a full-object PUT; Kubernetes itself enforces
`resourceVersion` matching. Piggybacking on the server's own check is simpler and more correct
than re-implementing conflict detection client- or backend-side, and matches how `kubectl apply`/
`kubectl replace` behave (per spec Assumptions, "apply" = full-object replace semantics for this
feature).

**Alternatives considered**:
- Strategic merge patch instead of full replace — rejected: spec explicitly scopes "apply" to the
  one opened resource with `kubectl apply`/`replace` semantics, not a merge; full replace also
  makes identity-field-change detection trivial (compare before parsing).
- Client-side diffing against a re-fetched copy to detect conflicts pre-emptively — rejected as
  redundant with the server's own 409; still computing a *visual* diff (see below) for the
  operator's diff-preview step, but that diff is a UX aid, not the conflict-detection mechanism.

**Diff-preview mechanism**: the frontend diffs the edited text buffer against `liveYaml`, which is
re-rendered (via the existing `jsonToYaml()` read path, `ui/src/resources/yaml.ts`) every time a
new `raw_json` arrives for the selected resource over Fathom's existing live resource-watch stream
— the same mechanism that already keeps the read-only YAML tab current with "no page reload" today.
**Decision (user-selected)**: the diff is live, not a static snapshot — if the cluster object
changes while the operator is still editing, the diff view updates to reflect the new live state
immediately, before apply. This surfaces drift earlier than waiting for the apply-time
`resourceVersion` check (FR-004) to catch it, at the cost of the diff base potentially shifting
under the operator mid-review; the UI must make it visually obvious when `liveYaml` has changed
since the diff was last reviewed (e.g. a "live content changed, re-review" indicator) so the
operator isn't confused by a diff that moved without their action. No new backend command or
polling loop is needed — the resource-watch stream already exists and already drives the current
read-only YAML tab.

**YAML crate choice**: `serde_yaml = "0.9"`. It is marked deprecated upstream (no further releases
planned) but remains the most mature, widely-used, and Serde-native YAML crate in the Rust
ecosystem today; alternatives (`serde_yaml_ng`, `serde_norway`) are community forks with far
smaller adoption and less audit history. Given Fathom only needs YAML→`serde_json::Value`
round-tripping (not advanced YAML features), `serde_yaml` 0.9 is pinned as a stable dependency per
the constitution's "dependencies SHOULD be stable" guidance; this choice should be revisited if
`serde_yaml` becomes unmaintained in a way that blocks a future k8s-openapi/kube-rs upgrade.

## R2 — Node action mechanism

**Decision**: Cordon/uncordon are a `Patch::Merge` against `Api<Node>` setting/clearing
`spec.unschedulable`. Taint add/remove is a `Patch::Merge` against `spec.taints` (full-array
replace computed backend-side from the current taint list). Drain is implemented as a background
task that lists the node's pods (excluding those already terminating), optionally filters out
DaemonSet-owned pods (spec FR-011), calls `Api<Pod>::evict()` for each remaining pod, and emits one
incremental progress event per pod outcome (evicted / blocked-by-PDB / failed), reusing the
`StreamEvent` + terminal `StreamClosed` convention `002-pod-logs-terminal` established for
logs/exec streaming. A companion `CancelDrainNode` command aborts the task's `JoinHandle`,
consistent with how exec/log session `JoinHandle`s are already tracked and aborted in
`src/backend/state.rs`.

**Rationale**: `Api<K>::evict(&self, name, ep)` (confirmed at
`kube-client-2.0.1/src/api/subresource.rs:464`) is the real Kubernetes Eviction subresource; a
blocked eviction due to a PodDisruptionBudget comes back as a 429 from the API server itself, so
PDB math is never reimplemented client- or backend-side (spec FR-010's "blocked-on" visibility is
just surfacing that 429's target pod). Bare-pod detection (spec edge case: refuse unless forced)
is a pre-check — list the node's pods and check `ownerReferences` for any pod with no controller
owner — before starting eviction, so a bare pod aborts the whole drain up front rather than mid-
stream.

Node-summary/detail read logic today lives inline in `src/backend/cmd/cluster.rs`
(`fetch_node_summaries`, `select_node`, both using `::kube::Api::<Node>::all(client.clone())`).
This plan promotes that logic into a new `src/backend/nodes.rs` module (mirroring the existing
`src/backend/pods.rs`/`flux.rs` per-domain module convention) and adds the four new mutating
functions there, with `cluster.rs`'s command dispatch delegating to it exactly as it already
delegates to `deployments`/`events`/`pods`/`sync` today (see the `use` list at
`src/backend/cmd/cluster.rs:8`).

**Alternatives considered**:
- Polling drain status instead of a push-progress stream — rejected: the codebase already solved
  "long-running backend operation with incremental UI feedback" for logs/exec; reusing that pattern
  is both simpler and constitution-aligned (no new mechanism invented).
- Reimplementing PDB-aware eviction ordering/backoff client-side — rejected per the plan-invocation
  instruction and confirmed unnecessary: the Eviction API already does this server-side.

## R3 — Generic CRD browsing mechanism

**Decision**: Use `kube::discovery` (`oneshot::group`/`pinned_group`, confirmed present in
`kube-client-2.0.1/src/discovery/mod.rs`) to enumerate `CustomResourceDefinition` objects via the
`apiextensions.k8s.io` API and build `ApiResource`/`ApiCapabilities` for each installed CRD kind.
Instances are listed/fetched through `kube_core::dynamic::DynamicObject` via
`Api::<DynamicObject>::all()`/`namespaced()` scoped dynamically from each CRD's discovered `Scope`.
Rendering reuses the existing generic Details/YAML/Events Inspector tab pattern (`Inspector.tsx`)
— a `DynamicObject`'s `data: serde_json::Value` already round-trips through the same `raw_json`
read-event shape every built-in kind uses, so no new tab/view code is needed beyond wiring the new
nav section and read commands.

**Rationale**: Both `kube::discovery` and `kube_core::dynamic::DynamicObject` are included in
kube-rs 2.0.1's `client` feature, which is already enabled by default (`fathom`'s `Cargo.toml` sets
`features = ["runtime", "derive", "ws"]`, and `client` is part of kube's own
`default = ["client", "ring"]` — confirmed via `kube-client-2.0.1/Cargo.toml`'s `[features]` block).
**No new dependency and no new feature flag are required.** This directly replaces the
hand-rolled, one-CRD-at-a-time approach `flux.rs`/`longhorn.rs` each take today with the generic
mechanism those two integrations lack, per the spec's explicit framing.

**Alternatives considered**:
- Hardcoding additional well-known CRD kinds (e.g. cert-manager, Prometheus Operator) one at a time
  — rejected: this is exactly the anti-pattern the spec is closing; Flux/Longhorn stay as
  hand-built integrations (out of scope to migrate them in this feature) but no *new* kind gets the
  same hardcoded treatment.
- A schema-aware form editor for CRD instances (reading the CRD's OpenAPI validation schema) —
  explicitly out of scope per spec Assumptions (read-only discovery/inspection only this feature).

## R4 — Protocol additions

**Decision**: New `BackendCmdV1` variants — `ApplyResourceYaml`, `CordonNode`, `UncordonNode`,
`DrainNode`, `CancelDrainNode`, `TaintNode`, `RefreshCrds`, `SelectCrdKind`, `RefreshCrdInstances`,
`SelectCrdInstance` — follow the exact enum-variant/field shape of the existing mutating commands
(`DeleteResource`, `ScaleWorkload`, `RestartWorkload` in `src/protocol/v1/cmd.rs`) for the first six,
and the existing read/select commands (`SelectNode`, and the Flux/Longhorn integration refresh
commands) for the last four. Every one of the six mutating variants gets an explicit arm in
`src/protocol/v1/mutation.rs`'s `mutation_target()` match returning `Namespaced { .. }` (apply,
drain — pod-scoped within a namespace) or `ClusterScoped` (cordon/uncordon/taint — node-scoped,
cluster-level), never `ReadOnly` and never a wildcard fallthrough (the match stays exhaustive by
construction, matching the file's existing "on purpose" design). The four CRD-read variants get
`ReadOnly` arms. New `StreamEvent`/`SystemEvent` variants (`ResourceApplied`, `NodeActionFinished`,
`DrainProgress`, `CrdCatalogUpdated`, `CrdInstancesUpdated`) follow existing shapes in
`src/protocol/v1/event.rs`.

**Rationale**: This is a direct application of Constitution Principle I (explicit mutation is
enforced structurally by the exhaustive `mutation_target()` match — a new mutating command literally
cannot compile without a classification decision) and Principle III (every new variant gets a
round-trip serialization test in `tests/protocol_serde.rs`, extending its existing per-variant test
table rather than inventing a new test style).

**Alternatives considered**: none seriously considered — the existing pattern is unambiguous and
the plan-invocation instructions explicitly directed following it.

## R5 — Frontend diff-viewer library

**Decision**: Add `diff` (the `jsdiff` package, `npm: diff`) as a new `ui/` dependency, using its
`diffLines()` function to compute a line-level diff between the live-YAML string and the
edited-YAML string, rendered with a small custom React component (`YamlDiffView.tsx`) styled with
the existing neon-noir token palette (no new CSS framework).

**Rationale**: `diff` is a minimal (no runtime dependencies), extremely widely-used
(50M+ weekly downloads), actively maintained pure-text-diffing library — it computes the diff data
structure only, leaving rendering to Fathom's own component so the diff view matches the app's
existing visual language exactly (the same reasoning `002-pod-logs-terminal`'s research.md applied
when justifying `xterm.js`: pick the minimal library that does the one hard part, build the rest
natively). No existing frontend dependency (`ui/package.json` audited: `@tanstack/react-table`,
`@tanstack/react-virtual`, `radix-ui`, `xterm`, `zustand`, `sonner`, `react-resizable-panels`,
`cmdk`, `recharts`) provides text diffing.

**Alternatives considered**:
- A full embedded code-editor-with-diff-mode library (e.g. Monaco, CodeMirror 6 merge view) —
  rejected as heavyweight for this use case: Fathom's YAML tab is a review-and-apply flow, not a
  general-purpose code editor; bundling a full editor engine contradicts Principle V (Fast and
  Quiet) for marginal benefit over a plain `<textarea>` + computed diff.
- Hand-rolling an LCS diff — rejected: reinventing a well-solved, well-tested problem for no
  benefit; `diff` is dependency-free and small.

## R6 — Confirmation-flow reuse

**Decision**: All five new mutating actions integrate with the existing
`ConfirmDialog`/`ActionDialogHost`/`runAction` flow (`ui/src/store/index.ts`,
`ui/src/store/actions.ts`) exactly as delete/scale/restart already do: the UI stages the action
(`showActionDialog`), nothing is sent until explicit confirmation, `runAction` then sends the
command and reduces `CommandFinished{ok, error}` into a toast. Drain's incremental progress
additionally streams `DrainProgress` events into a live progress view within the same confirmed-
action lifecycle (dialog stays open/updates until `StreamClosed`), mirroring how the Terminal tab's
exec session already keeps a confirmed action "live" via a stream rather than a single
request/response.

**Rationale**: This is the constitution's Principle I applied directly — no new confirmation UI is
built; the existing mechanism already generalizes to every new action here without modification.

**Alternatives considered**: none — inventing a parallel confirmation mechanism would violate
Principle I's intent (a single, consistent, unavoidable confirmation gate for every mutation).

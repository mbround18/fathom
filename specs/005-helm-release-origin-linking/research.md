# Research: Helm Release View & Origin-Context Linking

## R1: Decoding Helm's release-storage format

**Decision**: Read `Secret`/`ConfigMap` objects across all namespaces (or the active namespace
filter) whose `type` is `helm.sh/release.v1` (Secrets) or whose data carries the same marker
convention for ConfigMap storage, via `Api<Secret>::all(client)` / `Api<ConfigMap>::all(client)`
(the same all-namespaces listing shape `flux.rs::list_instances` and `applications.rs` already
use). Each object's single data key (Secret: key = release name, e.g. via the `owner=helm` /
`name`/`version` labels Helm itself stamps: `name`, `owner=helm`, `status`, `version`) holds the
release payload as base64 text; decoding is base64 → gzip → JSON, entirely server-side
(Constitution Principle II — raw bytes never reach the frontend, only the decoded structured
fields).

**Rationale**: This is Helm 3's own storage convention (no Tiller, no CLI shellout — FR-001). The
storage object's own labels (`name`, `owner`, `status`, `version`, `namespace` via the object's own
metadata) are enough to recover a degraded row (FR-014) even if the JSON payload fails to parse.

**Alternatives considered**: Shelling out to the `helm` binary — rejected per the spec's explicit
non-goal and because it would require the binary to be installed/on `PATH`, which Fathom cannot
assume (violates Principle II: cluster access must live in the backend as a `kube-rs` client, not
depend on an external CLI).

## R2: New dependency for gzip decode

**Decision**: `Cargo.toml` has `base64 = "0.22.1"` already (used elsewhere for encryption), which
covers the base64 half. It has **no** gzip/deflate crate. Add `flate2 = "1"` (default `miniz_oxide`
backend — no system zlib dependency, keeping Tier 1 builds free of a new host prerequisite,
consistent with `research.md`'s existing precedent in the promo-brochure Hubble work choosing
`protobuf-src` specifically to avoid a system `protoc`). This is the one new Rust dependency this
feature needs.

**Rationale**: Helm's release payload is gzip-compressed JSON; there is no way to decode it without
a gzip implementation. `flate2` is the de facto standard, actively maintained, non-pre-release.

**Alternatives considered**: Reimplementing DEFLATE by hand — rejected as needless risk for a
solved problem. `libflate` — less widely used than `flate2` in the Rust ecosystem; no reason to
prefer it here.

## R3: Origin-context linking — where Flux health data comes from

**Decision**: Do not add a new Flux poll/scan path. `src/backend/flux.rs::list_kustomizations_and_helmreleases`
and `summarize()` already exist (built for 004's attribution use) and already resolve exactly the
data FR-010 needs (`FluxReadyState`, message, revision, suspended, last_transition) for a Flux
`Kustomization` or `HelmRelease`. Add one new **on-demand, read-only** command
`SelectFluxResource { kind, namespace, name }` that re-runs this existing discovery/list call and
returns the single matching `FluxResourceSummary` (or a "not found" signal, FR-013) as a new
`ResourceReadEvent::FluxResourceDetailsUpdated(FluxResourceSummary)`. This is a fresh live read
each time the operator clicks an origin link (not a cache read of the dashboard's last poll), so
the health shown is always current — matching Principle II (backend owns the cluster, always
authoritative) rather than trusting a client-cached snapshot from whenever the Flux dashboard pane
last happened to poll.

**Rationale**: Reuses 100% of existing Flux discovery/parsing code (`find_served`,
`list_instances`, `summarize`, `classify_ready`) — zero new cluster-access logic, zero new poll
loop (`ensure_flux_poll` in `sync.rs` is untouched). The existing `RefreshFlux` command still
serves the dashboard aggregate view unchanged; this is a separate, narrower "get me just this one
object's status" read, mirroring how every other kind's `SelectX` command already works (fetch one
object on demand, not filter a cached list).

**Alternatives considered**: Having the frontend match against its already-cached
`store.flux.overview` from the last `FluxOverviewUpdated` event — rejected because that data can be
stale (the dashboard pane may not be open, or the last poll predates the very reconciliation
failure the operator is investigating), and because every other drill-down/detail read in this
codebase goes through a live backend command, not a client-side cache lookup — staying consistent
avoids a one-off exception to Principle II and to the existing `SelectX` convention.

## R4: A new "Flux resource" inspector kind, not a special case in Inspector.tsx

**Decision**: `Inspector.tsx` currently renders `<Empty title="Not ported yet">` for any
`ObjectRef.section` with no registered descriptor (`descriptorFor` returns `undefined`). An origin
link must therefore point at a real, minimal `ResourceDescriptor` — not a special-cased render
path — to fit the existing breadcrumb/back-stack (`pane.history: ObjectRef[]`) and
`RelatedResourceLink`'s generic `openInspector(relatedObjectRef(r), ...)` call unchanged. Add a new
section `flux-resource` (distinct from the existing `integrations/flux` dashboard-aggregate
section) with a small read-only descriptor: one "Details" tab showing
Ready/Failed/Reconciling/Suspended/Unknown, the message verbatim, revision, suspended flag, and
last transition — reusing `DetailSections.tsx`'s existing `fields` block type, no new block kind.
No YAML tab, no Events tab beyond what every descriptor gets by default from Inspector's generic
scaffolding (or explicitly suppressed if that scaffolding requires opt-in — resolved at
implementation time, not a structural decision here).

**Rationale**: Keeps the origin link on the exact same rails as every other related-resource link
(`RelatedResourceRef` → `relatedObjectRef` → `openInspector` → `descriptorFor` → `Inspector`) with
zero special-casing, per the plan's explicit instruction to reuse rather than invent a parallel
navigation pattern.

**Alternatives considered**: Rendering the Flux health inline inside the Helm release's own
`related` detail section (no separate inspector target) — rejected because it wouldn't satisfy
FR-012's breadcrumb/back-stack requirement or FR-013's "not found" state cleanly, and because a
Kustomization above the HelmRelease (a second hop) has nowhere to go without its own
inspectable target.

## R5: `RelatedResourceRef.kind` string collision between "our" HelmRelease and Flux's HelmRelease CRD

**Decision**: No protocol change needed. `RelatedResourceRef.section` (a `SectionKey`) is what
`Inspector.tsx` actually uses to resolve a descriptor (`descriptorFor(target.section)`); `kind` is
display-only plus per-descriptor row-matching. Fathom's own Helm release resource uses
`section: section_keys::helm_releases()`, `kind: "HelmRelease"`; a Flux origin link uses
`section: section_keys::flux_resource()`, `kind: "HelmRelease"` or `"Kustomization"` as
appropriate. Both can legitimately be labeled "HelmRelease" (that is genuinely each one's real
Kubernetes `kind`) without ambiguity, because `section` — not `kind` — is what routes navigation.

**Rationale**: This differs from 004's CRD-instance deviation (where `kind` alone, with no `group`,
truly was insufficient to disambiguate a click target) — here `section` already fully disambiguates,
so no `RelatedResourceRef` schema change is needed.

## R6: Helm release ownership of live workloads

**Decision**: Reuse `related.rs`'s existing `related_for_application`-style pattern: given a
release's `name`/`namespace`, find workloads it owns via the same Helm tracking labels
`applications.rs::attribute_from_meta` already checks (`app.kubernetes.io/managed-by=Helm`,
`meta.helm.sh/release-name`, `meta.helm.sh/release-namespace`), dispatched from `related_resources`
on `kind == "HelmRelease"` (Fathom's own section, not the Flux one). No new Kubernetes API calls
beyond what `applications.rs::list_owned_workloads`/`owned_related_resources_and_pods` already do —
call the same helper functions with a release-identity filter instead of an application-instance
filter.

**Rationale**: Exactly the "owns" relation vocabulary 004 established for
Deployment/StatefulSet/DaemonSet, applied with a different identity key. No new resolver logic
shape, just a new match arm in `related_resources()`.

**Alternatives considered**: A bespoke Helm-specific ownership resolver — rejected; the existing
`applications.rs` helpers already do exactly this label-based ownership discovery and are
generalized enough to reuse directly.

## R7: No new frontend dependency

**Decision**: Verified `ui/package.json` — the existing YAML/large-text view component used by
003's YAML tab (`jsonToYaml` in `ui/src/resources/yaml.ts`, rendered via `YamlEditTab`) and
`DetailSections.tsx`'s existing block types (`fields`, `list`, `map`) cover every rendering need
here (values, manifest, revision history, owned-resource links). No new viewer, table, or diff
library is needed.

**Rationale**: Values/manifest render as read-only YAML text (reuse `jsonToYaml` + a read-only
variant of the existing YAML tab, or the same tab in non-editable mode since `descriptor.rawJson`
being present is what currently turns on edit affordances in `Inspector.tsx` — for Helm releases,
either omit `rawJson` entirely, per Applications' own precedent of no editable-YAML-for-a-
synthesized-object, or provide `rawJson` but no corresponding mutation command so the Apply button
has nothing to submit to; exact choice resolved at implementation time, but no new dependency
either way). Revision history is a plain list (`DetailBlock` `list` type, like `related`).

## R8: Namespace scope for the release list

**Decision**: List Helm release-storage Secrets/ConfigMaps across all namespaces when no namespace
filter is active, and scope the `Api::namespaced(...)` listing per selected namespace otherwise —
identical to how every other namespaced resource list in this codebase already respects
`state.cluster.namespace_filter` (FR-003).

**Rationale**: No new filtering mechanism; this is the standard existing pattern used by
`Deployments`, `Pods`, etc.

# Feature Specification: Helm Release View & Origin-Context Linking

**Feature Branch**: `005-helm-release-origin-linking`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the Helm read view (docs/MVP.md §5a \"Read (near-term)\") plus origin-context linking from any Helm release to its owning GitOps controller, extending the §5b generic interlinking mechanism that shipped in 004-applications-interlinking. Read-only: list installed Helm releases (from Helm's own release-storage Secrets/ConfigMaps, not the helm binary), a release inspector (effective + user-supplied values, rendered manifest, revision history, owned resources), and — the operator's explicit ask — when a release traces back to a Flux HelmRelease/Kustomization, a clickable link from the release straight to that controller's live reconciliation health (Ready/Failed/Reconciling/Suspended, the real error message, revision, suspended flag), so 'why is this broken' is answered in two clicks instead of cross-referencing kubectl by hand. Non-goals: upgrade/rollback/uninstall mutation, values-editor, chart install/search, Argo CD reads (design the relation so Argo can plug in later), revision diffing."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Browse installed Helm releases (Priority: P1)

An operator opens a new "Helm" nav section and sees every Helm release installed across the
cluster (or the active namespace, respecting the existing namespace filter), with enough at-a-
glance information — chart, versions, revision, status, last updated — to spot a stuck or failed
release without opening anything.

**Why this priority**: This is the foundational list every other story in this spec depends on;
without it there is nothing to select into an inspector or drill from.

**Independent Test**: Point Fathom at a cluster with several Helm releases (some `deployed`, at
least one `failed` or `pending-upgrade`) and confirm the list shows one row per release with
correct chart/version/revision/status/last-updated, and that narrowing the namespace filter hides
releases outside it — mirrors the existing list-pane convention every other resource kind uses.

**Acceptance Scenarios**:

1. **Given** a cluster with three installed Helm releases in two namespaces, **When** the operator
   opens the Helm section with no namespace filter, **Then** all three releases are listed, each
   exactly once, with their chart name, chart version, app version, revision number, status, and
   last-updated time.
2. **Given** a release whose latest revision status is `failed`, **When** the operator views the
   list, **Then** that release's Status column visibly distinguishes it from `deployed` releases
   (e.g. a distinct status indicator), consistent with how failed/not-ready states are shown
   elsewhere in Fathom.
3. **Given** the namespace filter is narrowed to exclude a release's namespace, **When** the
   operator views the list, **Then** that release's row disappears, matching every other list in
   Fathom.

---

### User Story 2 - Trace a broken release to its GitOps origin (Priority: P1)

An operator investigating a broken or unexpectedly-configured Helm release wants to know
immediately whether a GitOps controller (Flux) owns it and, if so, whether *that* controller is
itself healthy — without leaving Fathom to run `kubectl get kustomization` or `flux get`.

**Why this priority**: This is the specific capability the operator most wants — most of this
team's releases arrive via Flux (and, for other teams, Argo or another orchestrator), so the
single highest-value thing this feature can do is answer "is this broken because of the chart, or
because the thing reconciling it is stuck?" in as few clicks as possible.

**Independent Test**: Install a Helm release via a Flux `HelmRelease`, then deliberately break the
Flux `HelmRelease`'s reconciliation (e.g. point its `sourceRef` at a nonexistent source). Open the
Helm release's inspector in Fathom and confirm a clickable link to the originating Flux resource
appears, and that clicking it shows the Flux resource's real `Failed` state and error message
inline — no separate CLI lookup needed. Separately, open a release with no Flux ownership markers
and confirm no such link is shown (not an error state).

**Acceptance Scenarios**:

1. **Given** a Helm release that was created and is reconciled by a Flux `HelmRelease` object,
   **When** the operator opens that release's inspector, **Then** a labeled, clickable link to the
   originating Flux `HelmRelease` (and, if resolvable, the `Kustomization` above it) is shown.
2. **Given** the originating Flux `HelmRelease` is currently `Failed`, **When** the operator clicks
   the origin link, **Then** the resulting view shows the Flux resource's ready state as `Failed`,
   its current status message verbatim, its last-attempted revision, and whether it is suspended —
   without requiring any further navigation.
3. **Given** the originating Flux `HelmRelease` is healthy (`Ready`), **When** the operator clicks
   the origin link, **Then** the resulting view clearly shows a healthy state, so a healthy origin
   is just as quick to confirm as a broken one.
4. **Given** a Helm release with no Flux (or other GitOps) ownership markers, **When** the operator
   opens that release's inspector, **Then** no origin link is shown, and nothing in the UI implies
   this is a problem or a missing feature.
5. **Given** the operator has drilled from the Helm release list into a release's inspector and
   then into its Flux origin, **When** they use the existing breadcrumb/back control, **Then** they
   return to the Helm release inspector, then to the Helm release list, retracing their path.

---

### User Story 3 - Inspect a release's full detail (Priority: P2)

An operator selects a specific Helm release to understand exactly what was installed: what values
produced the running configuration, what manifest was actually applied, what happened across past
revisions, and which live cluster resources this release currently owns.

**Why this priority**: This is the detailed-diagnosis layer once the operator already knows *which*
release to look at (from Story 1) and, often, whether its origin is the problem (Story 2) — valuable,
but secondary to being able to see the release exists and trace its origin at all.

**Independent Test**: Select a Helm release with a non-trivial values override and more than one
revision; confirm the inspector shows effective values (merged, including chart defaults) separately
from the values the operator/CI actually supplied, the exact manifest last applied, a list of past
revisions with their status/description/timestamp, and the workloads currently owned by the release,
each clickable into its own inspector.

**Acceptance Scenarios**:

1. **Given** a release installed with a custom values override for one setting the chart also
   defines a default for, **When** the operator views the inspector's values, **Then** both the
   effective (merged) value and the user-supplied override are visible and distinguishable.
2. **Given** a release's most recent revision, **When** the operator views the inspector's manifest
   tab, **Then** the exact YAML last applied by that revision is shown, read-only.
3. **Given** a release with three past revisions (one superseded, one failed, one deployed),
   **When** the operator views the inspector's revision history, **Then** all three are listed in
   order with their status, description, and timestamp.
4. **Given** a release that owns two Deployments, **When** the operator views the inspector's
   related/owned-resources list, **Then** both Deployments are listed and clicking either opens that
   Deployment's own inspector (reusing the existing drill-down mechanism).

---

### Edge Cases

- A Helm release-storage Secret/ConfigMap exists but its payload fails to decode (corrupted,
  truncated, or an unrecognized encoding): the release is still listed using whatever metadata is
  recoverable from the storage object's own labels (name, namespace, revision), with values/manifest
  shown as an explicit "unavailable" state rather than the list silently omitting the release or the
  whole view erroring.
- A release has zero resources currently matching its ownership markers (everything it once owned
  was deleted out-of-band): the release is still listed and its owned-resources section shows an
  explicit "no resources found" state, not an error.
- A release's Flux `HelmRelease` origin has itself been deleted after the Helm release was created
  (Flux ownership labels remain on the release, but the Flux object no longer exists): the origin
  link is shown but resolves to an explicit "not found" state when clicked, not a blank or frozen
  panel — consistent with the stale-link handling already established in 004-applications-interlinking.
- Two releases of the same name exist in different namespaces: both are listed distinctly, scoped
  correctly by namespace, with no cross-contamination of revision history or owned resources.
- A superseded or uninstalled revision's storage object still exists (Helm keeps history by
  default): it contributes to revision history but is not treated as the "current" release state
  shown in the list.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST list every installed Helm release visible to the current cluster
  connection, derived from Helm's own release-storage objects, not from a separately-run `helm`
  command-line invocation.
- **FR-002**: Each listed release MUST show its name, namespace, chart name, chart version, app
  version, current revision number, status (as recorded by Helm itself — e.g. deployed, failed,
  pending-upgrade, pending-install, pending-rollback, superseded, uninstalled), and last-updated
  time.
- **FR-003**: The release list MUST respect the same active namespace filter already used by every
  other resource list in Fathom.
- **FR-004**: Selecting a release MUST open its inspector in place (no route/page change), matching
  the existing pattern used by every other resource kind.
- **FR-005**: The release inspector MUST show the effective (merged) values and the user-supplied
  values as two distinguishable views, not conflated into one.
- **FR-006**: The release inspector MUST show the exact manifest content the release's current
  revision applied, read-only.
- **FR-007**: The release inspector MUST show a revision history: for each past revision, at least
  its number, status, description, and timestamp.
- **FR-008**: The release inspector MUST show the live cluster resources currently owned by the
  release, each rendered as a clickable link that opens that resource's own inspector via the
  existing related-resources drill-down mechanism.
- **FR-009**: When a Helm release was created and is being reconciled by a Flux `HelmRelease` object
  (detectable via the same Flux ownership markers already used elsewhere in Fathom to distinguish
  Flux-managed resources), the release's inspector MUST show a clickable link identifying that
  originating Flux resource.
- **FR-010**: Clicking the origin link MUST open a view showing that Flux resource's current
  reconciliation state (Ready/Failed/Reconciling/Suspended/Unknown), its status message verbatim,
  its current revision, and whether it is suspended — using data already available from Fathom's
  existing Flux integration, without requiring the operator to leave Fathom.
- **FR-011**: A Helm release with no detectable GitOps-controller ownership markers MUST NOT show an
  origin link, and this absence MUST NOT be presented as an error or degraded state.
- **FR-012**: The origin link and every owned-resource link MUST participate in the same
  breadcrumb/back-stack navigation already established for cross-resource drill-down, so the
  operator can retrace a path through a release, its origin, and back.
- **FR-013**: Following an origin link whose target no longer exists MUST show an explicit
  "not found" state in place of that link's destination, not a blank or frozen panel.
- **FR-014**: A release whose storage payload cannot be decoded MUST still appear in the list (using
  whatever metadata is recoverable) with an explicit "unavailable" indicator on the parts of its
  inspector that could not be decoded, rather than being silently omitted or crashing the view.
- **FR-015**: This feature MUST NOT introduce any way to upgrade, roll back, uninstall, or otherwise
  modify a Helm release, install a new one, or edit values — read/navigate only, consistent with how
  every other read-only view in Fathom behaves before its corresponding mutation feature (if any) is
  built separately.

### Key Entities

- **Helm Release**: A named, namespaced installation of a chart at a specific point in time — has an
  identity (name + namespace), a chart (name, version, app version), a current revision number, a
  Helm-reported status, effective and user-supplied values, a last-applied manifest, and zero or more
  live resources it currently owns.
- **Release Revision**: One historical entry in a release's upgrade/rollback history — has a revision
  number, status, description, and timestamp; distinct from the "current" release state shown in the
  list.
- **GitOps Origin Reference**: A reference from a Helm release to the GitOps controller resource (a
  Flux `HelmRelease`, and via it optionally a `Kustomization`) that created and reconciles it, carrying
  enough identity for the UI to resolve and display that controller's live health — modeled generically
  enough that a future non-Flux orchestrator (e.g. Argo CD) could supply the same kind of reference
  without redesigning this relationship.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can find any specific Helm release's chart, version, revision, and status
  within 10 seconds of opening the Helm section, without consulting a separate tool.
- **SC-002**: For a Helm release managed by Flux, an operator can determine whether the underlying
  GitOps reconciliation is healthy or failing, and see the actual error message if it's failing, in
  two clicks or fewer from the release's inspector.
- **SC-003**: An operator can distinguish a chart-default value from an explicitly user-supplied
  override for any given release setting without reading raw YAML by hand.
- **SC-004**: 100% of releases present in Helm's own release storage appear in the list, including
  ones whose storage payload cannot be fully decoded (degraded but present, never silently dropped).
- **SC-005**: Clicking an owned-resource or origin link never leaves the operator on a blank or
  indefinitely-loading panel, even when the link's target has since been deleted.

## Assumptions

- Helm 3 release-storage conventions apply (Secrets or ConfigMaps of type/labeled as
  `helm.sh/release.v1`, base64-encoded and gzip-compressed JSON payload) — Helm 2's Tiller-based
  storage is out of scope, consistent with this codebase targeting current-generation cluster
  tooling elsewhere (e.g. Flux, not the legacy `helm template`-and-commit pattern).
- "Origin" for v1 means Flux (`HelmRelease`/`Kustomization`), since that is the only GitOps
  integration Fathom has today; the relationship is modeled generically so Argo CD (or another
  orchestrator) can be added as a second origin source later without changing how the UI surfaces
  or navigates an origin link.
- Revision history is read-only and list-only for this spec — comparing two revisions' manifests or
  values side-by-side is deferred to whenever rollback mutation (a separate future spec) is built,
  reusing the diff-preview pattern already established for YAML-apply.
- "Resources currently owned by the release" reuses the same ownership-attribution and
  related-resources vocabulary already established for Applications (004) — a Helm release owning a
  Deployment/StatefulSet/DaemonSet is the same relationship shape, just keyed off release identity
  instead of application instance.
- Values and manifest content can be large for complex charts; rendering them reuses Fathom's
  existing large-text/YAML view component rather than a new viewer, so existing virtualization/perf
  characteristics apply unchanged.

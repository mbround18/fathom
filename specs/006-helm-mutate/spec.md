# Feature Specification: Helm Mutate (Upgrade, Rollback, Uninstall) with GitOps-Ownership Warnings

**Feature Branch**: `006-helm-mutate`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build the Helm mutate path (docs/MVP.md §5a \"Mutate\") on top of the read-only Helm view shipped in 005-helm-release-origin-linking. Scope: upgrade (re-apply current or edited values as a new revision, diff-previewed), rollback to a prior revision (diff-previewed), uninstall (confirmation-gated). All three actions go through the existing mutation.rs confirmation/guard mechanism from 003. Plus a mid-conversation addition: detect whether a Helm release is managed by Flux (or, in the future, Argo) and, when an operator is about to mutate it directly, warn them and link back to the owning GitOps source — since a manual change will likely be reverted on the controller's next reconciliation. Non-goals: chart install/search from a repo, a values-editor wizard/form UI, standalone revision-to-revision diffing outside the upgrade/rollback preview, multi-release bulk operations."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Roll back a release to a prior revision (Priority: P1)

An operator viewing a Helm release's revision history (shipped read-only in 005) notices the current revision is broken, picks an earlier known-good revision, previews exactly what will change in the cluster, and confirms the rollback.

**Why this priority**: Rollback is the highest-value, lowest-risk mutate action — it reuses Helm's own well-understood rollback semantics (no new values authoring, no chart resolution), and it is the action an operator reaches for first during an incident. It delivers standalone value even if upgrade/uninstall ship later.

**Independent Test**: Can be fully tested by selecting a release with 2+ revisions, choosing "Roll back to revision N", reviewing the manifest diff-preview, confirming, and observing the release's current revision counter increments with the target revision's manifest now live — without touching upgrade or uninstall code paths.

**Acceptance Scenarios**:

1. **Given** a release with revision history showing revisions 1-3, **When** the operator selects "Roll back" on revision 1, **Then** a diff-preview shows the rendered-manifest differences between the current live revision (3) and revision 1's manifest before anything is applied.
2. **Given** the diff-preview is showing, **When** the operator confirms, **Then** the rollback is applied, the release's revision count increases by one (rollback creates a new revision in Helm's own history, it does not delete revisions 2-3), and the new revision's manifest matches revision 1's.
3. **Given** the diff-preview is showing, **When** the operator cancels, **Then** no cluster state changes and the release is untouched.
4. **Given** a bare-Helm release with no GitOps ownership markers, **When** the operator opens the rollback confirmation, **Then** no GitOps warning is shown (absence, not an error state, matching 005's precedent).
5. **Given** a release whose origin traces to a Flux `HelmRelease` (or a future-supported Argo source), **When** the operator opens the rollback confirmation, **Then** a warning is shown stating the release is managed by that GitOps controller and that this manual rollback may be reverted the next time it reconciles, with a link to the owning GitOps resource (reusing 005's origin-link navigation).

---

### User Story 2 - Upgrade a release to new values or a new chart version (Priority: P2)

An operator wants to apply a values change (e.g. bump a resource limit) or move to a newer chart version without leaving the app, previews the exact resource diff the change would produce, and confirms.

**Why this priority**: Upgrade is the most commonly needed day-2 action but carries more risk than rollback (arbitrary new values/chart, not just replaying known-good history) and depends on the same diff-preview and GitOps-warning mechanism proven out by User Story 1, so it is sequenced second.

**Independent Test**: Can be fully tested by opening a release's inspector, choosing "Upgrade", editing the values YAML (reusing 003's existing YAML editor, no new form UI) and/or changing the chart version, reviewing the manifest diff-preview against the current live revision, confirming, and observing a new revision appear with the edited values/chart applied.

**Acceptance Scenarios**:

1. **Given** a release's current values, **When** the operator opens "Upgrade", **Then** the current effective values are pre-loaded into the existing YAML editor component for editing (no new structured-form UI).
2. **Given** edited values (or an unchanged values set, or a changed chart version), **When** the operator requests a preview, **Then** a diff-preview shows the rendered-manifest differences between the current live revision and what the upgrade would produce, computed without mutating anything.
3. **Given** the diff-preview is showing, **When** the operator confirms, **Then** the upgrade is applied as a new revision and the release's revision count increases by one.
4. **Given** edited values that fail to render (invalid YAML, template error), **When** the operator requests a preview, **Then** the system reports a clear error and does not apply anything, leaving the release unchanged.
5. **Given** a release whose origin traces to a GitOps controller, **When** the operator opens the upgrade flow, **Then** the same GitOps-ownership warning and origin link from User Story 1 is shown before the diff-preview step.

---

### User Story 3 - Uninstall a release (Priority: P3)

An operator decides a release is no longer needed and removes it, with an explicit confirmation step given the destructive, hard-to-reverse nature of the action.

**Why this priority**: Uninstall is the least frequently needed of the three actions day-to-day and, unlike upgrade/rollback, has no diff-preview to design (there is nothing to preview — the release and its resources are simply removed) — the lowest design complexity, sequenced last.

**Independent Test**: Can be fully tested by selecting "Uninstall" on a release, confirming through the destructive-action confirmation dialog, and observing the release disappear from the release list and its owned resources removed from the cluster (via Kubernetes' own owner-reference garbage collection once Helm's release-storage records are deleted).

**Acceptance Scenarios**:

1. **Given** a release, **When** the operator selects "Uninstall", **Then** a confirmation dialog appears requiring explicit confirmation (e.g. typing the release name or an equivalent deliberate-action affordance), matching the destructive-action pattern already used elsewhere in the app (e.g. node drain/delete).
2. **Given** the confirmation dialog, **When** the operator confirms, **Then** the release's Helm release-storage records are removed and the release no longer appears in the release list.
3. **Given** the confirmation dialog, **When** the operator cancels, **Then** the release and its resources are untouched.
4. **Given** a release whose origin traces to a GitOps controller, **When** the operator opens the uninstall confirmation, **Then** the same GitOps-ownership warning and origin link is shown — uninstalling a GitOps-managed release is likely to be recreated on the next reconciliation rather than staying removed, which is exactly the kind of surprise this warning exists to prevent.

---

### Edge Cases

- What happens when the release's underlying chart source is unreachable during an upgrade (e.g. `HelmRepository`/registry unavailable)? The upgrade preview/apply MUST fail with a clear, specific error rather than silently proceeding with stale or partial chart data.
- What happens if the target rollback revision's own stored manifest is itself corrupted/undecodable (mirrors 005's FR-014 degraded-payload handling)? The rollback action MUST be blocked with a clear error before any apply is attempted, not attempted against garbage data.
- What happens if the release changes state (e.g. another actor upgrades it) between when the diff-preview was computed and when the operator confirms? The apply MUST be computed fresh at confirm-time against the release's then-current state (or fail clearly if the assumed base revision no longer matches), never silently applying a stale preview.
- What happens for a release with no GitOps ownership markers at all? No warning is shown — this is the common, legitimate "managed directly by an operator/CI" case, not a gap to flag (consistent with 005's precedent for the origin-link's absence).
- What happens when GitOps ownership is detected but the owning controller resource itself can't currently be found/read (e.g. it was since deleted)? The warning MUST still state that GitOps ownership labels are present and a manual change may be reverted, even though the specific origin resource can't be linked to right now — the warning is driven by the release's own labels, not by successfully resolving the origin.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST let an operator roll back a Helm release to any prior revision visible in its revision history.
- **FR-002**: System MUST show a diff-preview (rendered-manifest differences between the release's current live revision and the target rollback revision) before a rollback is applied, and MUST NOT mutate cluster state while only previewing.
- **FR-003**: System MUST let an operator upgrade a Helm release by editing its values (via the existing YAML editor, no new structured-form UI) and/or changing its chart version.
- **FR-004**: System MUST show a diff-preview (rendered-manifest differences between the current live revision and what the upgrade would produce) before an upgrade is applied, and MUST NOT mutate cluster state while only previewing.
- **FR-005**: System MUST let an operator uninstall a Helm release, removing its release-storage records so Kubernetes garbage-collects the owned resources via their existing owner references.
- **FR-006**: Every one of upgrade, rollback, and uninstall MUST require an explicit, deliberate confirmation step before applying — none may be triggered by a single accidental click, following the same confirmation-gate pattern already used for other cluster-mutating actions in the app.
- **FR-007**: System MUST detect, for any Helm release, whether it is owned/managed by a GitOps controller (Flux today; the detection mechanism MUST be designed so a future Argo CD source can plug in without a redesign, mirroring 005's `GitOpsController` extensibility), reusing the origin-detection already built in 005 rather than adding a second detection path.
- **FR-008**: When a release is GitOps-managed, the confirmation step for upgrade, rollback, and uninstall MUST display an explicit warning that the change may be reverted the next time the owning controller reconciles, plus a link back to the owning GitOps resource (reusing 005's origin-link navigation) so the operator can go edit the actual source of truth instead.
- **FR-009**: The GitOps-ownership warning MUST be shown whenever GitOps ownership labels are present on the release, even if the specific owning controller resource can no longer be found/read — the warning does not depend on successfully resolving a live origin link.
- **FR-010**: System MUST NOT show any GitOps-ownership warning for a release with no GitOps ownership markers — absence is a normal, unflagged case, not an error or degraded state.
- **FR-011**: All Helm release-storage writes (upgrade, rollback, uninstall) MUST be performed by the backend; the frontend MUST NOT talk to the cluster directly, and MUST NOT receive raw release-storage bytes at any point in the mutate flow.
- **FR-012**: System MUST report a clear, specific error — and apply nothing — when a diff-preview or apply fails for any reason (unreachable chart source, invalid values/template error, corrupted target revision payload, stale base revision at confirm-time).
- **FR-013**: System MUST NOT provide chart install/search from a remote repository, a structured values-editor wizard/form UI, a standalone revision-to-revision diff browser independent of the upgrade/rollback preview, or multi-release bulk operations — all explicit non-goals for this feature.

### Key Entities

- **Helm Release Mutation Request**: An operator-initiated intent to upgrade, roll back, or uninstall a specific release — carries the release identity (namespace/name), the action kind, and, for upgrade, the proposed values/chart version.
- **Manifest Diff Preview**: The computed, side-by-side rendered-manifest difference between a release's current live state and the proposed post-action state (upgrade or rollback target) — shown to the operator before any apply, never persisted.
- **GitOps Ownership Warning**: Derived from the same origin-detection data introduced in 005 (`GitOpsOriginRef`/`GitOpsController`) — states which controller owns the release (when resolvable) and links to it; shown on every mutate confirmation for a GitOps-managed release regardless of whether the live origin resource itself is currently resolvable.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can roll back a release to a prior revision, from selecting the target revision to the rollback being live, in under 30 seconds of interaction (excluding cluster apply latency).
- **SC-002**: 100% of upgrade and rollback actions show a manifest diff-preview before any cluster mutation occurs — zero code paths allow a direct apply without a preview step.
- **SC-003**: 100% of upgrade, rollback, and uninstall actions require explicit confirmation before applying — zero single-click mutation paths.
- **SC-004**: 100% of mutate actions (upgrade/rollback/uninstall) attempted against a GitOps-managed release show the ownership warning and origin link before the operator can confirm; 0% of mutate actions against a non-GitOps-managed release show any such warning.
- **SC-005**: An operator can determine, before confirming any mutate action, whether their change is likely to be reverted by a GitOps controller — without needing to cross-reference `kubectl` or the Flux/Argo tooling by hand.

## Assumptions

- Diff-preview computation follows the same "render, don't apply" approach 003 already established for YAML-apply — Helm's own template-rendering logic (or an equivalent in-process render) is used to produce the proposed manifest without touching the cluster.
- The GitOps-ownership detection reuses 005's existing Flux-label-based detection and `GitOpsController`/`GitOpsOriginRef` shape as-is; no new cluster reads or watch loops are introduced solely for this warning.
- Argo CD ownership detection itself (as opposed to the extensibility of the `GitOpsController` shape to eventually support it) is out of scope for this feature, consistent with 005's own non-goal — Fathom has no Argo integration today.
- "Confirmation" for uninstall follows the same named-item confirmation dialog already used for other single-target destructive actions in the app (e.g. node drain) — it names the affected release by namespace/name but does not require typing the release name, since the app's existing confirmation component only requires typed input above a multi-item threshold that a single-release action never reaches.
- Upgrade's values editing surface is the existing raw YAML editor component from 003; no new structured/form-based values UI is built for this feature.

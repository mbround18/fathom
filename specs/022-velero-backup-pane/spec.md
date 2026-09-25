# Feature Specification: Velero Backup Pane

**Feature Branch**: `022-velero-backup-pane`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "Velero backup integration pane — detect Velero via its CRDs (velero.io API group: Backup, Schedule, Restore, BackupStorageLocation), show a read-only tool-pane listing backups/schedules/restores with status (completed/failed/in-progress), last-backup-age per schedule, and storage location health. Read-only, no mutation surface (no trigger-backup/restore action in this spec), no new external dependency."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See backup health at a glance (Priority: P1)

An operator responsible for a cluster's disaster-recovery posture opens Fathom and wants to know, without leaving the app, whether Velero is protecting the cluster and whether recent backups actually succeeded.

**Why this priority**: This is the core value of the feature — surfacing backup health inside the same tool the operator already uses for cluster inspection, instead of requiring a separate `velero` CLI session or a Kubernetes dashboard.

**Independent Test**: Can be fully tested by connecting Fathom to a cluster that has Velero installed with at least one completed and one failed Backup resource, opening the Velero pane, and confirming the list renders both backups with correct status and age. Delivers value on its own even before schedules/restores are added.

**Acceptance Scenarios**:

1. **Given** a cluster with Velero CRDs installed and one completed Backup and one failed Backup, **When** the operator opens the Velero pane, **Then** both backups are listed with their status clearly distinguished (completed vs. failed) and their age since completion/failure shown.
2. **Given** a Backup that is still running, **When** the operator opens the Velero pane, **Then** that backup is shown with an in-progress status distinct from completed/failed.
3. **Given** the operator is viewing the Velero pane, **When** the underlying cluster state changes (a new backup completes or an in-progress backup finishes), **Then** the pane updates to reflect the new state without requiring a manual page reload.

---

### User Story 2 - Confirm scheduled backups are current (Priority: P2)

An operator wants to confirm that recurring backup Schedules are actually producing backups on cadence, not silently failing to trigger.

**Why this priority**: A schedule that stopped firing is a common, easy-to-miss failure mode; catching a stale last-backup-age is second in value only to seeing backup results themselves.

**Independent Test**: Can be tested independently by pointing Fathom at a cluster with one or more Velero Schedule resources (each with a known last backup completion time) and confirming the pane shows each schedule with the correct last-backup age.

**Acceptance Scenarios**:

1. **Given** a cluster with a Velero Schedule that last produced a backup 2 hours ago, **When** the operator opens the Velero pane, **Then** the schedule is listed with a last-backup-age of approximately 2 hours.
2. **Given** a Velero Schedule that has never produced a backup, **When** the operator opens the Velero pane, **Then** the schedule is listed with an explicit "no backups yet" indication rather than a blank or misleading age.

---

### User Story 3 - Check restore history and storage location health (Priority: P3)

An operator investigating a recent incident wants to see what Restore operations have been run and whether the storage backend Velero depends on (object storage bucket) is currently reachable.

**Why this priority**: Restores are less frequent than backups, and storage-location health is a supporting signal rather than the primary workflow, but both are necessary for a complete read-only picture of backup posture.

**Independent Test**: Can be tested independently by pointing Fathom at a cluster with at least one Restore resource and one BackupStorageLocation, and confirming both render with their respective status/health.

**Acceptance Scenarios**:

1. **Given** a cluster with a completed Restore and a failed Restore, **When** the operator opens the Velero pane, **Then** both are listed with distinguishable status.
2. **Given** a BackupStorageLocation reporting "Available", **When** the operator opens the Velero pane, **Then** the storage location is shown as healthy.
3. **Given** a BackupStorageLocation reporting "Unavailable", **When** the operator opens the Velero pane, **Then** the storage location is shown as unhealthy with the reported reason, if any.

---

### User Story 4 - Understand when Velero is not installed (Priority: P1)

An operator on a cluster that does not run Velero opens the pane expecting either useful data or a clear explanation, not an error or a blank screen.

**Why this priority**: Many clusters Fathom connects to will not have Velero installed; an honest, non-alarming empty state is required for the feature to be trustworthy across the fleet of clusters operators manage, consistent with how the existing Flux GitOps panel behaves.

**Independent Test**: Can be tested independently by pointing Fathom at a cluster without the `velero.io` CRDs installed and confirming the pane shows a clear "Velero not installed" state with no errors logged.

**Acceptance Scenarios**:

1. **Given** a cluster without the `velero.io` CRDs installed, **When** the operator opens the Velero pane, **Then** the pane displays an explicit "Velero is not installed on this cluster" message instead of an empty list or an error.
2. **Given** Velero CRDs are installed but no Backup, Schedule, Restore, or BackupStorageLocation resources exist yet, **When** the operator opens the Velero pane, **Then** the pane shows Velero as detected but with an empty-results state per resource type (not conflated with "not installed").

---

### Edge Cases

- What happens when Velero CRDs are installed but the operator's RBAC identity lacks permission to list/read `velero.io` resources? The pane must show a clear permission-denied indication distinct from "not installed" and from a generic error.
- How does the pane handle a Backup or Restore stuck in-progress for an unusually long time (e.g., many hours)? It should still show "in-progress" with its elapsed running time rather than appearing stuck or timing out the UI.
- How does the pane handle multiple BackupStorageLocations, some healthy and some not? Each location must be shown individually with its own health rather than a single rolled-up status.
- What happens if Velero CRDs exist but are a version/shape Fathom doesn't recognize (e.g., a future CRD schema change)? The pane should degrade to showing raw/best-effort fields rather than crashing or hiding the resource entirely.
- What happens when the cluster connection drops while the pane is open? The pane should indicate stale/disconnected data rather than silently freezing on the last known state with no indication.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST detect whether Velero is installed on the currently connected cluster by checking for the presence of the `velero.io` CRD API group (specifically the `Backup`, `Schedule`, `Restore`, and `BackupStorageLocation` custom resource definitions).
- **FR-002**: System MUST present a dedicated, read-only Velero tool-pane that is reachable from the same navigation surface used for other cluster-integration panes (e.g., the existing Flux GitOps panel).
- **FR-003**: When Velero is not detected on the cluster, the pane MUST show an explicit "Velero not installed" empty state and MUST NOT show errors, blank lists, or partial/misleading data.
- **FR-004**: When Velero is detected, the pane MUST list Backup resources with, at minimum: name, namespace, status (completed / failed / in-progress / other Velero-reported phase), and a human-readable age relative to completion or start time.
- **FR-005**: When Velero is detected, the pane MUST list Schedule resources with, at minimum: name, namespace, schedule cadence, and the age since the last backup that schedule produced ("no backups yet" when none exist).
- **FR-006**: When Velero is detected, the pane MUST list Restore resources with, at minimum: name, namespace, the Backup they restore from, and status (completed / failed / in-progress / other Velero-reported phase).
- **FR-007**: When Velero is detected, the pane MUST list BackupStorageLocation resources with, at minimum: name, health/phase (e.g., Available / Unavailable), and the reported reason when unhealthy.
- **FR-008**: System MUST visually distinguish completed, failed, and in-progress states from one another for both Backups and Restores (e.g., via distinct status indicators), so an operator can identify failures at a glance without reading text closely.
- **FR-009**: System MUST keep the pane's data current by polling or watching the cluster in the background and refreshing displayed data without requiring the operator to manually reload, consistent with the live-update behavior of the existing Flux GitOps panel.
- **FR-010**: System MUST NOT provide any control in this pane that triggers, mutates, deletes, or otherwise changes a Backup, Schedule, Restore, or BackupStorageLocation (strictly read-only; no trigger-backup or trigger-restore action).
- **FR-011**: System MUST link Velero resources shown in the pane to the existing general-purpose resource Inspector so an operator can drill into full resource detail/YAML from the pane.
- **FR-012**: System MUST distinguish a permission-denied condition (RBAC identity lacks access to `velero.io` resources) from the "Velero not installed" state, and MUST surface it clearly to the operator rather than silently showing an empty list.
- **FR-013**: System MUST NOT introduce a new external runtime dependency (e.g., a Velero client library or additional service) to implement detection or data display; it MUST rely on the existing generic Kubernetes/CRD access already used by other integration panes.

### Key Entities

- **Backup**: A completed or in-progress point-in-time backup operation created by Velero; carries a status/phase, start/completion timestamps, and the namespaces/resources it covers.
- **Schedule**: A recurring backup policy that periodically produces new Backup resources; carries a cadence and a reference to its most recently produced Backup.
- **Restore**: An operation that restores cluster resources from a previously created Backup; carries a status/phase and a reference to the source Backup.
- **BackupStorageLocation**: The external object-storage location Velero uses to persist backup data; carries a health/availability phase and, when unhealthy, a reported reason.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can determine whether Velero is installed and, if so, whether the most recent backup succeeded, within 5 seconds of opening the Velero pane.
- **SC-002**: 100% of Backup and Restore resources present on a cluster at the time the pane is opened are shown with a correctly categorized status (completed / failed / in-progress) with no resource silently omitted.
- **SC-003**: Schedule last-backup-age is accurate to within the pane's refresh interval, so an operator never sees a schedule reported as current when its last backup is in fact stale beyond that interval.
- **SC-004**: On a cluster without Velero installed, 100% of pane visits show the "not installed" empty state rather than an error or blank screen.
- **SC-005**: Zero mutation actions (trigger backup, trigger restore, delete, edit) are reachable from the Velero pane in this release.

## Assumptions

- Velero is assumed to be installed with its standard CRDs under the `velero.io` API group; forks or heavily customized Velero installations that rename or omit these CRDs are out of scope for detection.
- The existing Flux GitOps panel's detect-CRD → live-watch/poll → tool-pane pattern (CRD presence check, background refresh, protocol event, frontend rendering, and Inspector linking) is the correct structural precedent to reuse for Velero, per direction from the feature owner.
- "Last-backup-age" for a Schedule is derived from the most recent Backup resource that references that Schedule (via owner reference or label), not from a separate Velero-reported "last backup time" field, since Velero does not guarantee such a field exists on every version.
- Read access to `velero.io` custom resources is governed by the same RBAC/kubeconfig context Fathom already uses for other cluster resources; no new credential or auth flow is introduced.
- Cross-cluster backup comparison and any backup/restore-triggering actions are explicitly out of scope for this feature and are deferred to a possible future mutation-surface spec.

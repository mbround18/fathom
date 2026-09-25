# Phase 0 Research: Velero Backup Pane

No `NEEDS CLARIFICATION` markers remained in the Technical Context — the feature owner's
instruction to mirror the existing Flux pane resolves the architectural questions directly. The
items below record the concrete decisions made while translating that pattern to Velero's CRDs.

## R1: CRD discovery strategy

- **Decision**: Discover `velero.io` CRDs dynamically at scan time via
  `Api<CustomResourceDefinition>::list`, look up the served version for each of
  `backups.velero.io`, `schedules.velero.io`, `restores.velero.io`, and
  `backupstoragelocations.velero.io`, then list instances as `kube::core::DynamicObject` —
  exactly the `find_served` / `list_instances` pattern in `src/backend/flux.rs`.
- **Rationale**: Works across Velero versions without pinning a compiled API/CRD schema, adds no
  new dependency (no `velero`/`k8s-openapi`-style typed crate exists or is needed), and is the
  explicitly mandated reuse target.
- **Alternatives considered**: A typed Velero API client crate — rejected, violates the "no new
  external dependency" constraint and the "don't invent a new architecture" instruction.

## R2: Backup/Restore status classification

- **Decision**: Map Velero's `status.phase` field (a string on Backup and Restore objects, e.g.
  `Completed`, `Failed`, `PartiallyFailed`, `InProgress`, `New`, `FailedValidation`) into a small
  closed enum: `Completed`, `Failed`, `InProgress`, `Unknown`. `PartiallyFailed` and
  `FailedValidation` are treated as `Failed` (spec FR-008 only requires distinguishing
  completed/failed/in-progress at a glance); `New`/`InProgress`/empty map to `InProgress`; any
  unrecognized value maps to `Unknown` so a future Velero phase never crashes or is silently
  dropped (spec edge case: unrecognized CRD shape must degrade gracefully).
- **Rationale**: Mirrors `FluxReadyState`'s closed-enum approach (`Ready`/`Failed`/
  `Reconciling`/`Suspended`/`Unknown`) for a status field read out of a dynamic object's JSON.
- **Alternatives considered**: Passing the raw phase string straight through with no
  normalization — rejected because FR-008 requires a clear visual distinction, and a raw string
  can't drive a stable color/tone mapping in the frontend.

## R3: Last-backup-age for a Schedule

- **Decision**: For each Schedule, find the most recent Backup whose
  `metadata.labels["velero.io/schedule-name"]` equals the Schedule's name (this label is written
  by Velero on every Backup it creates from a Schedule), take that Backup's
  `status.completionTimestamp` (falling back to `status.startTimestamp` if completion is absent,
  e.g. still in progress), and compute age as now − that timestamp. If no matching Backup exists,
  report "no backups yet" (FR-005, User Story 2 edge case).
- **Rationale**: Velero does not guarantee a `status.lastBackup` field on Schedule across all
  versions; deriving from the label is the same "don't trust a single version's status shape"
  caution applied by Flux's `revision` field (which also has a fallback: `lastAppliedRevision` →
  `lastAttemptedRevision`).
- **Alternatives considered**: Reading a `status.lastBackup`-style field directly if present —
  kept as a fast path only when present, with the label-based derivation as the reliable fallback
  used whenever it's absent.

## R4: BackupStorageLocation health

- **Decision**: Read `status.phase` (`Available` / `Unavailable` / absent) and
  `status.message` (present when unavailable) directly off the dynamic object, same shape as
  Flux's `ready_condition` helper reads `status.conditions`.
- **Rationale**: BackupStorageLocation's status shape is simpler than the `Ready`-condition
  pattern (no conditions array) — just phase + optional message — so no condition-array parsing
  is needed here.

## R5: RBAC-denied vs. not-installed

- **Decision**: `find_served` (CRD lookup) failing to find `velero.io` CRDs at all → "not
  installed" (`installed: false`, matching Flux's `installed` flag semantics). A CRD-list call
  itself failing outright, or a per-kind `list_instances` call returning a Forbidden/Unauthorized
  API error, is surfaced as a distinct backend error via `events::notify_error` (never silently
  swallowed into an empty list) — the frontend pane then must render a `PermissionDenied` state
  distinct from `installed: false`, per FR-012.
- **Rationale**: Flux's `list_instances` today swallows all listing errors into an empty `Vec`
  (`.unwrap_or_default()`), which — for permission errors specifically — would satisfy Flux's own
  spec but does not satisfy this feature's explicit FR-012 requirement to distinguish
  "not installed" from "denied." Velero's `list_instances` equivalent therefore returns a
  `Result` per kind (rather than swallowing errors like Flux's helper does), and CRD discovery
  success + at least one per-kind list failure with a Forbidden/Unauthorized status is classified
  as `VeleroOverview.permission_denied: bool` in addition to `installed: bool`.
- **Alternatives considered**: Reusing Flux's exact `list_instances` signature (swallow-to-empty)
  — rejected because it cannot satisfy FR-012 without this small, targeted deviation; scoped
  narrowly to error classification only, not a general architecture change.

## R6: Linking to the resource Inspector

- **Decision**: Velero resources are plain CRDs, so they already have a generic path into the
  Inspector: the existing custom-resources browser (`section_keys::custom_resources()`,
  `SelectCrdKind` / `SelectCrdInstance` / `RefreshCrdInstances` commands, `CrdInstanceDetails`
  event) already renders any CRD kind's full spec/status. The Velero pane's rows link out to that
  same generic CRD inspector view (selecting the resource's kind/namespace/name), rather than
  building a dedicated Velero object descriptor/inspector.
- **Rationale**: Satisfies FR-011 ("link ... to the existing resource Inspector") using the
  mechanism that already exists for arbitrary CRDs, instead of adding a second, parallel
  Inspector code path — keeping with "don't invent a new architecture."
- **Alternatives considered**: A dedicated `ObjectRef`/resource descriptor for each Velero kind
  (like first-class resources such as Pods have) — rejected as unnecessary weight for a read-only
  pane; the generic CRD inspector already covers full detail/YAML viewing.

## R7: Poll cadence and manual refresh

- **Decision**: Reuse the exact 20s `tokio::time::interval` background poll pattern from
  `poll_flux_overview`/`poll_hubble_traffic`, gated the same way (`ensure_velero_poll`, started
  from the same call site as `ensure_flux_poll` in `src/backend/cmd/cluster.rs`), plus a
  `RefreshVelero` command the frontend sends on pane mount (mirroring `FluxPanel`'s
  `useEffect(() => refreshFlux(), [refreshFlux])`) so opening the pane doesn't wait out the rest
  of the interval.
- **Rationale**: Directly mandated by the feature owner ("background poll/watch task"); matches
  SC-003's "accurate to within the pane's refresh interval" success criterion.

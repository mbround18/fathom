# Phase 1 Data Model: Velero Backup Pane

All types live in `src/protocol/v1/velero.rs`, derive `Serialize, Deserialize, TS` (ts-rs) exactly
like `src/protocol/v1/flux.rs`, and are exported to `ui/src/protocol/generated/*.ts` by the
existing bindings-generation step. Field types below are the Rust shapes; the generated TS
bindings mirror them 1:1 (ts-rs).

## VeleroPhase (enum)

Normalized status shared by Backup and Restore (see research.md R2).

| Variant | Meaning |
|---|---|
| `Completed` | Backup/Restore finished successfully. |
| `Failed` | Backup/Restore failed, partially failed, or failed validation. |
| `InProgress` | Backup/Restore is new or currently running. |
| `Unknown` | Phase missing or not recognized (forward-compat with future Velero phases). |

## VeleroBackupSummary

| Field | Type | Source |
|---|---|---|
| `name` | `String` | `metadata.name` |
| `namespace` | `String` | `metadata.namespace` |
| `phase` | `VeleroPhase` | `status.phase`, normalized |
| `schedule_name` | `Option<String>` | `metadata.labels["velero.io/schedule-name"]` |
| `start_timestamp` | `String` | `status.startTimestamp` (RFC3339), empty if absent |
| `completion_timestamp` | `String` | `status.completionTimestamp` (RFC3339), empty if absent/in-progress |
| `errors` | `i64` | `status.errors` (count), 0 if absent |
| `warnings` | `i64` | `status.warnings` (count), 0 if absent |

## VeleroScheduleSummary

| Field | Type | Source |
|---|---|---|
| `name` | `String` | `metadata.name` |
| `namespace` | `String` | `metadata.namespace` |
| `cron_schedule` | `String` | `spec.schedule` |
| `paused` | `bool` | `spec.paused`, default false |
| `last_backup_completion` | `Option<String>` | derived per research.md R3; `None` means "no backups yet" |
| `last_backup_phase` | `Option<VeleroPhase>` | phase of the most recent matching Backup, if any |

## VeleroRestoreSummary

| Field | Type | Source |
|---|---|---|
| `name` | `String` | `metadata.name` |
| `namespace` | `String` | `metadata.namespace` |
| `phase` | `VeleroPhase` | `status.phase`, normalized |
| `backup_name` | `String` | `spec.backupName` |
| `start_timestamp` | `String` | `status.startTimestamp`, empty if absent |
| `completion_timestamp` | `String` | `status.completionTimestamp`, empty if absent/in-progress |

## VeleroStorageLocationSummary

| Field | Type | Source |
|---|---|---|
| `name` | `String` | `metadata.name` |
| `available` | `bool` | `status.phase == "Available"` |
| `phase` | `String` | `status.phase`, raw (`Available`/`Unavailable`/empty) |
| `message` | `String` | `status.message`, empty if absent |
| `is_default` | `bool` | `spec.default`, default false |

## VeleroOverview (aggregate root, one instance per cluster connection)

| Field | Type | Notes |
|---|---|---|
| `installed` | `bool` | `false` when none of the four `velero.io` CRDs are found — drives the "not installed" empty state (FR-003). |
| `permission_denied` | `bool` | `true` when CRDs are present but at least one kind's list call returned Forbidden/Unauthorized (research.md R5, FR-012). |
| `backups` | `Vec<VeleroBackupSummary>` | Sorted by namespace, name. |
| `schedules` | `Vec<VeleroScheduleSummary>` | Sorted by namespace, name. |
| `restores` | `Vec<VeleroRestoreSummary>` | Sorted by namespace, name. |
| `storage_locations` | `Vec<VeleroStorageLocationSummary>` | Sorted by name. |

## State transitions

This is a read-only, point-in-time snapshot type (like `FluxOverview`) — there is no
client-side state machine. Each background poll tick (or manual `RefreshVelero`) produces a fresh
`VeleroOverview` that fully replaces the previous one in the frontend store; individual resources
don't have modeled transitions beyond the `VeleroPhase` values a Backup/Restore may report over
its lifetime (`InProgress` → `Completed` | `Failed`).

## Frontend store shape (`ui/src/store/index.ts`)

Mirrors the existing `flux: { overview: FluxOverview | null }` slice:

```ts
velero: {
  overview: VeleroOverview | null
}
refreshVelero: () => void   // sends BackendCmd::RefreshVelero
```

Populated by handling `SystemEvent.VeleroOverviewUpdated` in
`ui/src/store/handlers/system.ts`, the same event-dispatch module that already handles
`FluxOverviewUpdated` for the Tauri/React frontend (the `src/app/mailbox.rs` handler is the
legacy egui UI's analog and is not touched by this feature — see plan.md Constitution Check on
the frozen egui UI).

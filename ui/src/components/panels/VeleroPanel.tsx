/**
 * Velero backup tool pane (022-velero-backup-pane): renders `store.velero.overview`
 * (SystemEvent::VeleroOverviewUpdated) through `IntegrationSummary`. State arrives from a 20s
 * background scan (backend `sync.rs::ensure_velero_poll`); opening the pane also sends
 * `RefreshVelero`. Read-only; rows click through to the generic CRD Inspector (FR-011).
 */
import { ArchiveIcon } from '@phosphor-icons/react'
import { useEffect } from 'react'

import type {
  VeleroBackupSummary,
  VeleroOverview,
  VeleroPhase,
  VeleroRestoreSummary,
  VeleroScheduleSummary,
  VeleroStorageLocationSummary,
} from '@/protocol/types'
import { formatAge } from '@/resources/format'
import { useStore } from '@/store'

import { IntegrationSummary, type SummaryRow, type SummaryTile } from './IntegrationSummary'
import { Empty, MetricsUnavailable } from '../shell/PaneStates'

export const VELERO_PHASE_TONE: Record<VeleroPhase, SummaryRow['tone']> = {
  Completed: 'ok',
  Failed: 'err',
  InProgress: 'warn',
  Unknown: 'muted',
}

/** Backups/Restores: completion time, else start time (still in progress). */
function ageOf(completion: string, start: string): string {
  return formatAge(completion || start)
}

export function backupRow(b: VeleroBackupSummary): SummaryRow {
  return {
    key: `Backup/${b.namespace}/${b.name}`,
    testId: 'velero-backup-row',
    primary: `Backup ${b.namespace}/${b.name}`,
    secondary: `${b.phase} · ${ageOf(b.completion_timestamp, b.start_timestamp)} ago`,
    tone: VELERO_PHASE_TONE[b.phase],
  }
}

export function scheduleRow(s: VeleroScheduleSummary): SummaryRow {
  const last = s.last_backup_completion
    ? `last backup ${formatAge(s.last_backup_completion)} ago`
    : 'no backups yet'
  return {
    key: `Schedule/${s.namespace}/${s.name}`,
    testId: 'velero-schedule-row',
    primary: `Schedule ${s.namespace}/${s.name}`,
    secondary: `${s.cron_schedule}${s.paused ? ' (paused)' : ''} · ${last}`,
    tone: s.last_backup_phase ? VELERO_PHASE_TONE[s.last_backup_phase] : 'muted',
  }
}

export function restoreRow(r: VeleroRestoreSummary): SummaryRow {
  return {
    key: `Restore/${r.namespace}/${r.name}`,
    testId: 'velero-restore-row',
    primary: `Restore ${r.namespace}/${r.name}`,
    secondary: `from ${r.backup_name} · ${r.phase} · ${ageOf(r.completion_timestamp, r.start_timestamp)} ago`,
    tone: VELERO_PHASE_TONE[r.phase],
  }
}

export function storageLocationRow(l: VeleroStorageLocationSummary): SummaryRow {
  return {
    key: `BackupStorageLocation/${l.name}`,
    testId: 'velero-location-row',
    primary: `Storage ${l.name}`,
    secondary: l.available ? l.phase || 'Available' : l.message || l.phase || 'Unavailable',
    tone: l.available ? 'ok' : 'err',
  }
}

export function buildTiles(o: VeleroOverview): SummaryTile[] {
  const count = (p: VeleroPhase) => o.backups.filter((b) => b.phase === p).length
  return [
    { label: 'Completed', value: String(count('Completed')), tone: 'ok' },
    {
      label: 'Failed',
      value: String(count('Failed')),
      tone: count('Failed') > 0 ? 'err' : 'muted',
    },
    { label: 'In progress', value: String(count('InProgress')), tone: 'warn' },
    { label: 'Schedules', value: String(o.schedules.length), tone: 'muted' },
    { label: 'Restores', value: String(o.restores.length), tone: 'muted' },
    {
      label: 'Locations',
      value: `${String(o.storage_locations.filter((l) => l.available).length)}/${String(o.storage_locations.length)}`,
      tone: o.storage_locations.every((l) => l.available) ? 'ok' : 'err',
    },
  ]
}

export function VeleroPanel() {
  const overview = useStore((s) => s.velero.overview)
  const refreshVelero = useStore((s) => s.refreshVelero)
  const selectCrdKind = useStore((s) => s.selectCrdKind)
  const navigate = useStore((s) => s.navigate)

  useEffect(() => {
    refreshVelero()
  }, [refreshVelero])

  if (!overview) {
    return <MetricsUnavailable title="Scanning" detail="Waiting for the Velero scan to complete." />
  }
  if (!overview.installed) {
    return (
      <Empty
        title="Velero is not installed"
        detail="Velero is not installed on this cluster (no velero.io CRDs found)."
      />
    )
  }
  if (overview.permission_denied) {
    return (
      <Empty
        title="Permission denied"
        detail="Velero is installed, but this cluster connection doesn't have permission to read its backup resources."
      />
    )
  }

  const open = (kind: string, plural: string, scope: 'Namespaced' | 'Cluster') => () => {
    selectCrdKind({
      group: 'velero.io',
      kind,
      plural,
      versions: ['v1'],
      scope,
      permission_denied: false,
    })
    navigate('custom-resources')
  }
  const withOpen = (rows: SummaryRow[], kind: string, plural: string): SummaryRow[] =>
    rows.map((r) => ({ ...r, onSelect: open(kind, plural, 'Namespaced') }))

  const rows: SummaryRow[] = [
    ...withOpen(overview.backups.map(backupRow), 'Backup', 'backups'),
    ...withOpen(overview.schedules.map(scheduleRow), 'Schedule', 'schedules'),
    ...withOpen(overview.restores.map(restoreRow), 'Restore', 'restores'),
    ...overview.storage_locations.map(storageLocationRow).map((r) => ({
      ...r,
      onSelect: open('BackupStorageLocation', 'backupstoragelocations', 'Namespaced'),
    })),
  ]

  return (
    <IntegrationSummary
      icon={ArchiveIcon}
      title="Velero"
      tiles={buildTiles(overview)}
      rows={rows}
      emptyLabel="Velero is installed, but has no backups, schedules, restores, or storage locations yet."
    />
  )
}

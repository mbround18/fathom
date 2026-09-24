/** 022-velero-backup-pane fixtures: hand-built `VeleroOverview` payloads. */
import type { VeleroOverview } from '../../src/protocol/types'

const hoursAgo = (h: number) => new Date(Date.now() - h * 3600_000).toISOString()

export const VELERO_HEALTHY_AND_FAILED: VeleroOverview = {
  installed: true,
  permission_denied: false,
  backups: [
    {
      name: 'nightly-ok',
      namespace: 'velero',
      phase: 'Completed',
      schedule_name: 'nightly',
      start_timestamp: hoursAgo(2),
      completion_timestamp: hoursAgo(2),
      errors: 0,
      warnings: 0,
    },
    {
      name: 'adhoc-bad',
      namespace: 'velero',
      phase: 'Failed',
      schedule_name: null,
      start_timestamp: hoursAgo(5),
      completion_timestamp: hoursAgo(5),
      errors: 3,
      warnings: 0,
    },
  ],
  schedules: [],
  restores: [],
  storage_locations: [],
}

export const VELERO_NOT_INSTALLED: VeleroOverview = {
  installed: false,
  permission_denied: false,
  backups: [],
  schedules: [],
  restores: [],
  storage_locations: [],
}

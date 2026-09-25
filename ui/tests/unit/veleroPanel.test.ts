import { describe, expect, it } from 'vitest'

import {
  backupRow,
  restoreRow,
  scheduleRow,
  storageLocationRow,
  VELERO_PHASE_TONE,
} from '@/components/panels/VeleroPanel'
import type { VeleroBackupSummary } from '@/protocol/types'

const ago = (h: number) => new Date(Date.now() - h * 3600_000).toISOString()

const backup: VeleroBackupSummary = {
  name: 'b1',
  namespace: 'velero',
  phase: 'Completed',
  schedule_name: null,
  start_timestamp: ago(3),
  completion_timestamp: ago(2),
  errors: 0,
  warnings: 0,
}

describe('VeleroPanel logic', () => {
  it('maps phases to tones', () => {
    expect(VELERO_PHASE_TONE).toEqual({
      Completed: 'ok',
      Failed: 'err',
      InProgress: 'warn',
      Unknown: 'muted',
    })
  })

  it('builds backup rows with tone and age (falls back to start when in progress)', () => {
    expect(backupRow(backup)).toMatchObject({ tone: 'ok' })
    expect(backupRow(backup).secondary).toContain('2h ago')
    const running = backupRow({ ...backup, phase: 'InProgress', completion_timestamp: '' })
    expect(running.tone).toBe('warn')
    expect(running.secondary).toContain('3h ago')
  })

  it('renders schedule age or "no backups yet"', () => {
    const base = { name: 's', namespace: 'velero', cron_schedule: '0 0 * * *', paused: false }
    const withBackup = scheduleRow({
      ...base,
      last_backup_completion: ago(2),
      last_backup_phase: 'Completed',
    })
    expect(withBackup.secondary).toContain('last backup 2h ago')
    const none = scheduleRow({ ...base, last_backup_completion: null, last_backup_phase: null })
    expect(none.secondary).toContain('no backups yet')
  })

  it('maps restore phase and storage-location health', () => {
    const r = restoreRow({
      name: 'r',
      namespace: 'velero',
      phase: 'Failed',
      backup_name: 'b1',
      start_timestamp: ago(1),
      completion_timestamp: ago(1),
    })
    expect(r.tone).toBe('err')
    expect(r.secondary).toContain('from b1')
    const loc = storageLocationRow({
      name: 'l',
      phase: 'Unavailable',
      available: false,
      message: 'bucket unreachable',
      is_default: false,
    })
    expect(loc.tone).toBe('err')
    expect(loc.secondary).toBe('bucket unreachable')
  })
})

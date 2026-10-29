import { describe, expect, it } from 'vitest'

import {
  availableText,
  countStates,
  exclusionText,
  filterServices,
  intervalText,
  runningText,
  shortDigest,
  showUpdates,
  STATE_LABEL,
  STATE_ORDER,
  stacksOf,
  untilText,
  withChecks,
} from '@/features/updates/model'
import type { Me, ServiceUpdate, UpdateSettings, UpdateState } from '@/protocol/generated'

function su(stack: string, service: string, state: UpdateState, over: Partial<ServiceUpdate> = {}): ServiceUpdate {
  return {
    stack,
    service,
    reference: `${service}:latest`,
    running: { image_id: 'sha256:abcdef0123456789', repo_digest: null, version: null, revision: null },
    available: null,
    state,
    reason: null,
    effective: { policy: 'notify', source: { kind: 'default' }, excluded: null, labels: [] },
    last_checked: null,
    last_outcome: null,
    replicas: 1,
    ...over,
  }
}

const SERVICES = [su('web', 'app', 'available'), su('web', 'db', 'up_to_date'), su('blog', 'ghost', 'available')]

describe('updates model', () => {
  it('labels and orders every state', () => {
    expect(new Set(STATE_ORDER).size).toBe(Object.keys(STATE_LABEL).length)
    expect(STATE_LABEL.check_failed).toBe("Can't check")
  })

  it('filters by state and stack', () => {
    expect(filterServices(SERVICES, 'all', '')).toHaveLength(3)
    expect(filterServices(SERVICES, 'available', '').map((s) => s.service)).toEqual(['app', 'ghost'])
    expect(filterServices(SERVICES, 'available', 'web').map((s) => s.service)).toEqual(['app'])
    expect(countStates(SERVICES)).toEqual({ available: 2, up_to_date: 1 })
    expect(stacksOf(SERVICES)).toEqual(['blog', 'web'])
  })

  it('shortens digests and describes images', () => {
    expect(shortDigest('sha256:0123456789abcdef')).toBe('0123456789ab')
    expect(shortDigest(null)).toBe('—')
    expect(runningText(su('a', 'b', 'up_to_date'))).toBe('abcdef012345')
    expect(runningText(su('a', 'b', 'not_checked', { running: null }))).toBe('no container')
    const avail = su('a', 'b', 'available', {
      available: {
        digest: 'sha256:ffffeeeeddddcccc',
        platform_digest: '',
        config_digest: '',
        version: '2.0',
        revision: null,
        created: null,
        user: null,
      },
    })
    expect(availableText(avail)).toBe('2.0 (ffffeeeedddd)')
    expect(availableText(su('a', 'b', 'up_to_date'))).toBeNull()
  })

  it('words exclusions and intervals', () => {
    expect(exclusionText({ kind: 'depths_itself' })).toContain('never updates itself')
    expect(exclusionText({ kind: 'scope_mismatch', scope: 'prod' })).toContain('scope prod')
    expect(intervalText(3600)).toBe('every hour')
    expect(intervalText(7200)).toBe('every 2 hours')
  })

  it('changes only checks and the interval', () => {
    const s: UpdateSettings = {
      checks_enabled: false,
      interval_secs: 21_600,
      window: { days: ['mon'], start: '03:00', end: '05:00' },
      label_only: false,
      cleanup: false,
      health_timeout_secs: 120,
    }
    expect(withChecks(s, true, 3600)).toEqual({ ...s, checks_enabled: true, interval_secs: 3600 })
    expect(withChecks(s, true)).toEqual({ ...s, checks_enabled: true })
  })

  it('shows the page only to admins with the feature on', () => {
    const me = { role: 'admin', features: { shell: false, edit: false, host_metrics: false, files: 'off', updates: true } } as unknown as Me
    expect(showUpdates(me)).toBe(true)
    expect(showUpdates({ ...me, role: 'user' })).toBe(false)
    expect(showUpdates({ ...me, features: { ...me.features, updates: false } })).toBe(false)
    expect(showUpdates(null)).toBe(false)
  })

  it('says how long until the next check', () => {
    const now = Date.parse('2026-10-28T12:00:00Z')
    expect(untilText('2026-10-28T12:00:30Z', now)).toBe('in under a minute')
    expect(untilText('2026-10-28T12:30:00Z', now)).toBe('in 30 min')
    expect(untilText('2026-10-28T15:00:00Z', now)).toBe('in 3 h')
  })
})

/** Spec 062 US2–US5: the pure helpers behind batches, runs, policies, windows, labels and cleanup. */
import { describe, expect, it } from 'vitest'

import { detailLines } from '@/features/audit/model'
import {
  isUtcZone,
  labelEffectText,
  labelName,
  outcomeText,
  outcomeTone,
  runEntries,
  settingsProblem,
  sourceText,
  triggerText,
  windowProblem,
  windowText,
} from '@/features/updates/model'
import type { AuditEntry, UpdateRun, UpdateSettings } from '@/protocol/generated'

const alice = { provider: 'github' as const, subject: '1', username: 'alice' }
const bob = { provider: 'github' as const, subject: '2', username: 'bob' }

function entry(over: Partial<AuditEntry> & Pick<AuditEntry, 'action'>): AuditEntry {
  return {
    at: '2026-10-28T03:01:00Z',
    actor: alice,
    stack: 'web',
    preview_id: null,
    operation_id: null,
    argv: [],
    target: null,
    diff_summary: null,
    outcome: { kind: 'succeeded' },
    source_ip: '127.0.0.1',
    ...over,
  }
}

const SETTINGS: UpdateSettings = {
  checks_enabled: true,
  interval_secs: 21_600,
  window: { days: ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'], start: '03:00', end: '05:00' },
  label_only: false,
  cleanup: false,
  health_timeout_secs: 120,
}

describe('runs', () => {
  it('words every outcome, and tones failures red', () => {
    expect(outcomeText({ kind: 'updated' })).toBe('Updated')
    expect(outcomeText({ kind: 'skipped', reason: 'busy: alice deploy' })).toBe('Skipped: busy: alice deploy')
    expect(outcomeText({ kind: 'rollback_failed' })).toBe('Rollback failed')
    expect(outcomeText({ kind: 'stale' })).toBe('Skipped: the stack changed, preview again')
    expect(outcomeTone({ kind: 'rollback_failed' })).toContain('text-status-err')
    expect(outcomeTone({ kind: 'rolled_back' })).toContain('text-status-warn')
    expect(outcomeTone({ kind: 'updated' })).toContain('text-status-ok')
  })

  it('says who started a run, and who a scheduled run acted for', () => {
    expect(triggerText({ kind: 'manual', actor: alice })).toBe('by alice')
    expect(triggerText({ kind: 'scheduled', on_behalf_of: { web: alice, blog: bob, db: alice } })).toBe(
      'scheduled, on behalf of alice, bob',
    )
    expect(triggerText({ kind: 'scheduled', on_behalf_of: {} })).toBe('scheduled')
  })

  it("matches a run's own entries, and removals inside its window", () => {
    const run: UpdateRun = {
      run_id: 'r1',
      trigger: { kind: 'manual', actor: alice },
      stacks: [],
      started_at: '2026-10-28T03:00:00Z',
      finished_at: '2026-10-28T03:05:00Z',
    }
    const update = { run_id: 'r1', services: [], rejected_digest: null, hold: null }
    const mine = entry({ action: 'image_update', update })
    const other = entry({ action: 'image_update', update: { ...update, run_id: 'r2' } })
    const removed = entry({ action: 'image_remove' })
    const later = entry({ action: 'image_remove', at: '2026-10-28T04:00:00Z' })
    const elsewhere = entry({ action: 'image_remove', stack: 'blog' })
    expect(runEntries(run, 'web', [mine, other, removed, later, elsewhere])).toEqual([mine, removed])
    expect(runEntries({ ...run, finished_at: null }, 'web', [later])).toEqual([later])
  })
})

describe('windows and settings', () => {
  it('words a window', () => {
    expect(windowText(SETTINGS.window)).toBe('every day 03:00–05:00')
    expect(windowText({ days: ['sun', 'sat'], start: '23:00', end: '02:00' })).toBe('Sat, Sun 23:00–02:00')
  })

  it('refuses an empty, malformed or zero-length window', () => {
    expect(windowProblem(SETTINGS.window)).toBeNull()
    expect(windowProblem({ ...SETTINGS.window, days: [] })).toBe('Choose at least one day.')
    expect(windowProblem({ ...SETTINGS.window, start: '24:00' })).toBe('Times are HH:MM, 24-hour.')
    expect(windowProblem({ ...SETTINGS.window, end: '03:00' })).toBe('The window ends when it starts: choose a different end.')
  })

  it('checks the health timeout, the scope and the window', () => {
    expect(settingsProblem(SETTINGS)).toBeNull()
    expect(settingsProblem({ ...SETTINGS, health_timeout_secs: 10 })).toBe('The health timeout is 30 to 1800 seconds.')
    expect(settingsProblem({ ...SETTINGS, health_timeout_secs: 1801 })).toBe('The health timeout is 30 to 1800 seconds.')
    expect(settingsProblem({ ...SETTINGS, scope: 'media' })).toBeNull()
    expect(settingsProblem({ ...SETTINGS, scope: 'a b' })).toBe('A scope is letters, digits, dot, dash or underscore.')
    expect(settingsProblem({ ...SETTINGS, window: { ...SETTINGS.window, days: [] } })).toBe('Choose at least one day.')
  })

  it('recognises the UTC fallback zone', () => {
    expect(isUtcZone('UTC (no /etc/localtime)')).toBe(true)
    expect(isUtcZone('Europe/Berlin')).toBe(false)
  })
})

describe('Watchtower labels', () => {
  it('names where a policy comes from', () => {
    expect(sourceText({ kind: 'stack_policy' })).toBe('stack policy')
    expect(sourceText({ kind: 'default' })).toBe('default')
    expect(sourceText({ kind: 'label', rule: 'monitor_only' })).toBe('monitor-only label')
    expect(sourceText({ kind: 'label', rule: 'no_pull' })).toBe('no-pull label')
  })

  it('words each effect, and strips the label prefix', () => {
    expect(labelEffectText({ kind: 'applied' })).toBe('applied')
    expect(labelEffectText({ kind: 'not_run', reason: 'order comes from the compose file' })).toBe(
      'not run: order comes from the compose file',
    )
    expect(labelEffectText({ kind: 'not_used', reason: "compose's stop_signal applies" })).toBe("not used: compose's stop_signal applies")
    expect(labelEffectText({ kind: 'unrecognised' })).toBe('not recognised: ignored')
    expect(labelName('com.centurylinklabs.watchtower.lifecycle.pre-update')).toBe('lifecycle.pre-update')
    expect(labelName('com.centurylinklabs.watchtower')).toBe('com.centurylinklabs.watchtower')
  })
})

describe('audit details for updates', () => {
  it('lists the services changed and who a scheduled run acted for', () => {
    const e = entry({
      action: 'image_update',
      on_behalf_of: bob,
      update: {
        run_id: 'r1',
        services: [
          {
            service: 'app',
            from_image: 'nginx:1.29',
            from_digest: 'sha256:1111aaaa2222bbbb',
            to_image: null,
            to_digest: 'sha256:9999ffff8888eeee',
            version: '1.29.1',
          },
        ],
        rejected_digest: null,
        hold: null,
      },
    })
    expect(detailLines(e)).toEqual(['app: nginx:1.29 (1111aaaa2222) -> nginx:1.29 (9999ffff8888), 1.29.1', 'on behalf of bob'])
  })

  it('shows a removal as the command and what it freed', () => {
    const e = entry({
      action: 'image_remove',
      argv: [['docker', 'image', 'rm', 'sha256:1111']],
      diff_summary: 'removed sha256:1111 (nginx@sha256:1111)',
    })
    expect(detailLines(e)).toEqual(['docker image rm sha256:1111', 'removed sha256:1111 (nginx@sha256:1111)'])
  })
})

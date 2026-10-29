/**
 * Spec 061 T050: the update status reducer, the card's state for each `state` and `outcome`, the
 * navigation dot rule (data-model.md "Dot dismissal") and the admin line's wording.
 */
import { describe, expect, it } from 'vitest'

import {
  ADMIN_LINE,
  adminLine,
  canUpdate,
  cardState,
  reduce,
  runFailed,
  shortDigest,
  shortRevision,
  showDot,
  when,
} from '@/features/settings/updates'
import type { AvailableImage, Me, RunInfo, UpdateOutcome, UpdateStatus, UpdateStatusLine } from '@/protocol/wire'

const OUTCOMES: UpdateOutcome[] = ['up_to_date', 'update_available', 'older_build', 'rebuilt', 'cant_tell']
const RUNNING_DIGEST = `sha256:${'1'.repeat(64)}`
const AVAILABLE_DIGEST = `sha256:${'2'.repeat(64)}`

// Kept here rather than imported from tests/support, which is typed for the browser-side e2e harness.
const available = (over: Partial<AvailableImage> = {}): AvailableImage => ({
  digest: AVAILABLE_DIGEST,
  version: '0.3.0',
  revision: 'fedcba9876543',
  built_at: '2026-10-27T12:00:00Z',
  release_notes: null,
  release_notes_note: null,
  ...over,
})

const updateFixtures = {
  available,
  status: (outcome: UpdateOutcome = 'update_available', over: Partial<UpdateStatus> = {}): UpdateStatus => ({
    enabled: true,
    image: 'docker.io/mbround18/keel:latest',
    state: 'checked',
    outcome,
    cant_tell_reason: null,
    checked_at: '2026-10-28T12:00:00Z',
    next_check_at: '2026-10-28T18:00:00Z',
    manual_after: null,
    last_error: null,
    running: { version: '0.2.0', revision: 'abc1234def5678', built_at: null, digest: RUNNING_DIGEST, display: '0.2.0' },
    available: available(outcome === 'up_to_date' ? { digest: RUNNING_DIGEST } : {}),
    digest_compare: true,
    update_path: { kind: 'run' },
    update_run: null,
    ...over,
  }),
  off: (): UpdateStatus => ({ ...updateFixtures.status('up_to_date'), enabled: false, state: 'not_checked', outcome: null, available: null }),
  notChecked: (): UpdateStatus => ({ ...updateFixtures.off(), enabled: true }),
  failed: (code: string): UpdateStatus => ({
    ...updateFixtures.status('up_to_date'),
    last_error: { code, at: '2026-10-28T12:00:00Z' },
  }),
  run: (id: string, digest: string): RunInfo =>
    ({ id, state: 'queued', params: { keel_server_image_digest: digest } }) as unknown as RunInfo,
}

function statusLine(s: UpdateStatus): UpdateStatusLine {
  return { update_available: s.outcome === 'update_available', checked_at: s.checked_at }
}

function me(instance_role: Me['instance_role'], update_dot_dismissed: string | null = null): Me {
  return { instance_role, update_dot_dismissed } as unknown as Me
}

describe('reduce', () => {
  it('an UpdateStatus replaces what is held', () => {
    const status = updateFixtures.status()
    expect(reduce({ status: null, line: null }, { type: 'UpdateStatus', data: status })).toEqual({ status, line: null })
  })
  it('an UpdateStatusLine replaces it with the line only', () => {
    const line = statusLine(updateFixtures.status())
    expect(reduce({ status: updateFixtures.status(), line: null }, { type: 'UpdateStatusLine', data: line })).toEqual({ status: null, line })
  })
  it('anything else leaves it as it was', () => {
    const held = { status: updateFixtures.off(), line: null }
    expect(reduce(held, { type: 'Confirmed', data: { preview_id: 'p' } })).toBe(held)
  })
})

describe('cardState', () => {
  it('off when checks are off, whatever else it says', () => {
    expect(cardState(updateFixtures.off())).toBe('off')
    expect(cardState({ ...updateFixtures.status(), enabled: false })).toBe('off')
  })
  it('not checked yet, and checking', () => {
    expect(cardState(updateFixtures.notChecked())).toBe('not_checked')
    expect(cardState({ ...updateFixtures.status(), state: 'checking' })).toBe('checking')
  })
  it.each(OUTCOMES)('the outcome %s once checked', (outcome) => {
    expect(cardState(updateFixtures.status(outcome))).toBe(outcome)
  })
  it('a failed check keeps the last result', () => {
    expect(cardState(updateFixtures.failed('network'))).toBe('up_to_date')
  })
})

describe('canUpdate', () => {
  it.each([
    ['update_available', true],
    ['older_build', true],
    ['rebuilt', true],
    ['up_to_date', false],
    ['cant_tell', false],
  ] as const)('%s → %s', (outcome, can) => {
    expect(canUpdate(updateFixtures.status(outcome))).toBe(can)
  })
  it('never with checks off', () => {
    expect(canUpdate(updateFixtures.off())).toBe(false)
  })
})

describe('the dot: role == owner && outcome == update_available && available.digest != me.update_dot_dismissed', () => {
  it('shows for an owner with an update available', () => {
    expect(showDot(me('owner'), updateFixtures.status('update_available'))).toBe(true)
  })
  it('hides once that digest is dismissed, and returns for another', () => {
    expect(showDot(me('owner', AVAILABLE_DIGEST), updateFixtures.status('update_available'))).toBe(false)
    const newer = updateFixtures.status('update_available', { available: updateFixtures.available({ digest: `sha256:${'3'.repeat(64)}` }) })
    expect(showDot(me('owner', AVAILABLE_DIGEST), newer)).toBe(true)
  })
  it.each(['older_build', 'rebuilt', 'cant_tell', 'up_to_date'] as const)('no dot for %s', (outcome) => {
    expect(showDot(me('owner'), updateFixtures.status(outcome))).toBe(false)
  })
  it('never for admins, users, or before the status arrives', () => {
    expect(showDot(me('admin'), updateFixtures.status())).toBe(false)
    expect(showDot(me('user'), updateFixtures.status())).toBe(false)
    expect(showDot(me('owner'), null)).toBe(false)
    expect(showDot(null, updateFixtures.status())).toBe(false)
  })
})

describe('the admin line', () => {
  it('says an owner can update, and nothing else', () => {
    expect(ADMIN_LINE).toBe('A newer Keel is available; an owner can update it.')
    expect(adminLine({ update_available: true, checked_at: null })).toBe(ADMIN_LINE)
  })
  it('is absent without an update', () => {
    expect(adminLine({ update_available: false, checked_at: '2026-10-28T12:00:00Z' })).toBeNull()
    expect(adminLine(null)).toBeNull()
  })
})

describe('formatting', () => {
  it('short values and UTC minutes', () => {
    expect(shortDigest(AVAILABLE_DIGEST)).toBe(`sha256:${'2'.repeat(12)}`)
    expect(shortRevision('abc1234def5678')).toBe('abc1234')
    expect(shortRevision('abc1234def5678-dirty')).toBe('abc1234-dirty')
    expect(when('2026-10-28T12:00:00Z')).toBe('2026-10-28 12:00 UTC')
  })
  it('a run that ended without updating', () => {
    const run = updateFixtures.run('r', AVAILABLE_DIGEST)
    expect(runFailed(run)).toBe(false)
    expect(runFailed({ ...run, state: 'failed' })).toBe(true)
    expect(runFailed({ ...run, state: 'succeeded' })).toBe(false)
    expect(runFailed(null)).toBe(false)
  })
})

/**
 * Spec 064 T010: which variant of the approval panel a person sees on a held run (contracts/ui.md).
 */
import { describe, expect, it } from 'vitest'

import { approvalView, relative, waitingText } from '@/features/runs/approval'
import type { ApprovalEntry, Me, RunApproval, RunInfo, TeamRole } from '@/protocol/wire'

const WS = 'ws-1'
const ME_ID = 'u-me'

function me(instance_role: Me['instance_role'], role?: TeamRole): Me {
  const grants = role ? [{ workspace: WS, workspace_slug: 'ops', role, profiles: [] }] : []
  return { id: ME_ID, instance_role, grants } as unknown as Me
}

function entry(user: string): ApprovalEntry {
  return { user, name: user, role: 'approver', at: '2026-10-27T12:00:00Z', reason: null }
}

function run(over: Partial<RunInfo> = {}, approval: Partial<RunApproval> = {}): RunInfo {
  return {
    workspace: WS,
    state: 'pending_approval',
    requested_by: null,
    approval: { needed: 1, four_eyes: false, expires_at: '2026-10-28T12:00:00Z', approvals: [], denial: null, ...approval },
    ...over,
  } as RunInfo
}

const APPROVER = me('user', 'approver')

describe('approvalView', () => {
  it('offers Approve and Deny to someone who can approve', () => {
    expect(approvalView({ run: run(), me: APPROVER })).toEqual({ kind: 'act' })
    expect(approvalView({ run: run(), me: me('user', 'ws_admin') })).toEqual({ kind: 'act' })
    expect(approvalView({ run: run(), me: me('owner') })).toEqual({ kind: 'act' })
  })

  it('is restricted for people without Approve in the workspace', () => {
    expect(approvalView({ run: run(), me: me('user', 'viewer') })).toEqual({ kind: 'restricted' })
    expect(approvalView({ run: run(), me: me('user', 'operator') })).toEqual({ kind: 'restricted' })
    expect(approvalView({ run: run(), me: null })).toEqual({ kind: 'restricted' })
  })

  it('leaves the four-eyes starter only Deny', () => {
    const held = run({ requested_by: ME_ID }, { four_eyes: true })
    expect(approvalView({ run: held, me: APPROVER })).toEqual({ kind: 'requester' })
  })

  it('lets the starter approve when the policy has no four-eyes, and anyone approve a push run', () => {
    expect(approvalView({ run: run({ requested_by: ME_ID }), me: APPROVER })).toEqual({ kind: 'act' })
    expect(approvalView({ run: run({ requested_by: null }, { four_eyes: true }), me: APPROVER })).toEqual({ kind: 'act' })
  })

  it('says answered once the person has approved', () => {
    const held = run({}, { needed: 2, approvals: [entry(ME_ID)] })
    expect(approvalView({ run: held, me: APPROVER })).toEqual({ kind: 'answered' })
    expect(approvalView({ run: run({}, { needed: 2, approvals: [entry('u-other')] }), me: APPROVER })).toEqual({ kind: 'act' })
  })

  it('is closed once the run has left pending_approval, whoever looks', () => {
    for (const state of ['queued', 'running', 'denied', 'expired', 'cancelled'] as const) {
      expect(approvalView({ run: run({ state }), me: APPROVER })).toEqual({ kind: 'closed' })
      expect(approvalView({ run: run({ state }), me: me('user', 'viewer') })).toEqual({ kind: 'closed' })
    }
  })
})

describe('waitingText', () => {
  it('reads "Waiting · k of N" for a held run and nothing otherwise', () => {
    expect(waitingText(run({}, { needed: 2, approvals: [entry('u-1')] }))).toBe('Waiting · 1 of 2')
    expect(waitingText(run())).toBe('Waiting · 0 of 1')
    expect(waitingText(run({ state: 'queued' }))).toBeNull()
    expect(waitingText(run({ approval: null }))).toBeNull()
  })
})

describe('relative', () => {
  const now = Date.parse('2026-10-27T12:00:00Z')
  it('reads the largest whole unit, either way', () => {
    expect(relative('2026-10-28T11:00:00Z', now)).toBe('in 23 hours')
    expect(relative('2026-10-29T12:00:00Z', now)).toBe('in 2 days')
    expect(relative('2026-10-27T11:55:00Z', now)).toBe('5 minutes ago')
    expect(relative('2026-10-26T12:00:00Z', now)).toBe('yesterday')
    expect(relative('2026-10-27T12:00:30Z', now)).toBe('just now')
  })
})

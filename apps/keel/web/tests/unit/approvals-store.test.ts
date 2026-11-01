/**
 * Spec 064 T024: the count of runs waiting for the person's approval. It is read once `Me` arrives
 * and again on reconnect, follows `RunState` pushes at most once per 500 ms, and stays at 0 for
 * someone who can approve nowhere.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useApprovals } from '@/features/runs/approvals-store'
import type { Me } from '@/protocol/wire'

import { FakeSocket } from './fake-socket'
import { frames, lastCmd, ME, resetSession, signIn, timers } from './session-helpers'

const APPROVER: Me = {
  ...ME,
  instance_role: 'user',
  grants: [{ workspace: 'ws-1', workspace_slug: 'ops', role: 'approver', profiles: [] }],
}

function counts(s: FakeSocket): number {
  return frames(s).filter((f) => f.cmd?.type === 'CountPendingApprovals').length
}

function answer(s: FakeSocket, counts: { workspace: string; count: number }[]) {
  const cmd = lastCmd(s, 'CountPendingApprovals')
  if (!cmd) throw new Error('no CountPendingApprovals was sent')
  s.serverSend({ reply_to: cmd.id, event: { type: 'PendingApprovals', data: { counts } } })
}

function runState(s: FakeSocket, state: string) {
  s.serverSend({ event: { type: 'RunState', data: { run: 'run-1', state, reason: null } } })
}

beforeEach(() => {
  resetSession()
  useApprovals.setState({ total: 0, byWorkspace: {} })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useApprovals', () => {
  it('asks once Me arrives, and totals the workspaces', () => {
    const s = signIn(APPROVER)
    expect(counts(s)).toBe(1)
    answer(s, [
      { workspace: 'ws-1', count: 2 },
      { workspace: 'ws-2', count: 1 },
    ])
    expect(useApprovals.getState().total).toBe(3)
    expect(useApprovals.getState().byWorkspace).toEqual({ 'ws-1': 2, 'ws-2': 1 })
  })

  it('asks again after a reconnect', () => {
    const s = signIn(APPROVER)
    answer(s, [{ workspace: 'ws-1', count: 1 }])
    s.serverClose(1001)
    timers.shift()?.()
    const s2 = FakeSocket.last()
    expect(s2).not.toBe(s)
    s2.open()
    expect(counts(s2)).toBe(1)
  })

  it('debounces RunState pushes by 500 ms, and ignores states that change nothing', () => {
    const s = signIn(APPROVER)
    answer(s, [{ workspace: 'ws-1', count: 2 }])
    vi.useFakeTimers()
    runState(s, 'pending_approval')
    runState(s, 'queued')
    runState(s, 'running')
    vi.advanceTimersByTime(499)
    expect(counts(s)).toBe(1)
    vi.advanceTimersByTime(1)
    expect(counts(s)).toBe(2)
    runState(s, 'succeeded')
    vi.advanceTimersByTime(1000)
    expect(counts(s)).toBe(2)
  })

  it('a total of 0 empties the count', () => {
    const s = signIn(APPROVER)
    answer(s, [{ workspace: 'ws-1', count: 1 }])
    useApprovals.getState().refresh()
    answer(s, [])
    expect(useApprovals.getState().total).toBe(0)
    expect(useApprovals.getState().byWorkspace).toEqual({})
  })

  it('does not ask for someone who can approve nowhere', () => {
    const s = signIn({ ...ME, instance_role: 'user', grants: [{ workspace: 'ws-1', workspace_slug: 'ops', role: 'viewer', profiles: [] }] })
    expect(counts(s)).toBe(0)
    expect(useApprovals.getState().total).toBe(0)
  })
})

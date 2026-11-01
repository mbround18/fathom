/**
 * How many held runs wait for this person's approval (spec 064 US4), for the chip on the Runs nav
 * item. The server counts only what the person could answer now: workspaces where they may
 * approve, less runs they answered, four-eyes runs they started and runs past their window.
 *
 * Read once `Me` says who this is (and again when it changes), on reconnect, after the person
 * answers a run, and at most once per 500 ms while `RunState` pushes move runs in or out of
 * `pending_approval`. Someone who can approve nowhere is never asked about: their count is 0.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { approvesSomewhere } from '@/features/hosts/workspaces'
import type { RequestId, RunPhase } from '@/protocol/wire'

import { onAnswered } from './decision'

export interface Approvals {
  total: number
  /** Workspace id → its count; workspaces with nothing waiting are absent. */
  byWorkspace: Record<string, number>
  refresh: () => void
}

/** States a held run enters or leaves by; anything else cannot change the count. */
const MOVES: ReadonlySet<RunPhase> = new Set<RunPhase>(['pending_approval', 'queued', 'denied', 'expired', 'cancelled'])
const DEBOUNCE_MS = 500

const EMPTY = { total: 0, byWorkspace: {} }

let asked: RequestId | null = null
let timer: ReturnType<typeof setTimeout> | null = null

export const useApprovals = create<Approvals>()(() => ({
  ...EMPTY,
  refresh: () => {
    if (timer !== null) clearTimeout(timer)
    timer = null
    if (!approvesSomewhere(useSession.getState().me)) {
      asked = null
      useApprovals.setState(EMPTY)
      return
    }
    asked = sendCmd({ type: 'CountPendingApprovals' })
  },
}))

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined && reply_to === asked) {
    asked = null
    // A refusal keeps the last count: the chip is a hint, and the next push asks again.
    if (event.type !== 'PendingApprovals') return
    const byWorkspace: Record<string, number> = {}
    let total = 0
    for (const { workspace, count } of event.data.counts) {
      if (count <= 0) continue
      byWorkspace[workspace] = count
      total += count
    }
    useApprovals.setState({ total, byWorkspace })
    return
  }
  if (event.type === 'RunState' && MOVES.has(event.data.state) && approvesSomewhere(useSession.getState().me)) {
    if (timer !== null) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      useApprovals.getState().refresh()
    }, DEBOUNCE_MS)
  }
})

onReconnected(() => {
  useApprovals.getState().refresh()
})

onAnswered(() => {
  useApprovals.getState().refresh()
})

useSession.subscribe((s, prev) => {
  if (s.me !== prev.me) useApprovals.getState().refresh()
})
// Loaded after `Me` already arrived.
if (useSession.getState().me) useApprovals.getState().refresh()

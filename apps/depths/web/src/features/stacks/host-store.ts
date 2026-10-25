/**
 * Host metrics (`WatchHost`, admin only), fed by the shared socket.
 *
 * - `watchHost()` is reference counted: the first caller sends `WatchHost`, the last one to let go
 *   sends `UnwatchHost`. A hidden page lets go too, and watches again when it is shown.
 * - Each `HostSample` becomes `latest` and one point in a ring of `HOST_HISTORY` per figure, for
 *   the sparklines. The server sends no history: it starts empty on every page load.
 * - A `Denied` reply (not an admin, or `DEPTHS_HOST_PROC` unset) is kept and stops the watch.
 * - A reconnect re-sends the watch; leaving the signed-in state clears everything.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { DenyReason, HostSample, RequestId } from '@/protocol/generated'

import { HOST_HISTORY, hostPoint, type HostPoint } from './metrics'

export interface HostState {
  latest: HostSample | null
  /** Oldest first, at most `HOST_HISTORY`. */
  history: HostPoint[]
  denied: DenyReason | null
}

const INITIAL: HostState = { latest: null, history: [], denied: null }

export const useHost = create<HostState>()(() => ({ ...INITIAL }))

/** The signed-in person is an admin and the server reads the host's `/proc`. */
export function useCanSeeHost(): boolean {
  return useSession((s) => s.me?.role === 'admin' && s.me.features.host_metrics)
}

let holders = 0
/** The server has (or is being asked for) a watch on this socket. */
let watching = false
let request: RequestId | null = null

function pageVisible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden'
}

function sync(): void {
  const want = holders > 0 && pageVisible() && useHost.getState().denied === null
  if (want === watching) return
  if (want) {
    const id = sendCmd({ type: 'WatchHost' })
    // No socket yet: try again when it opens.
    if (id === null) return
    request = id
    watching = true
  } else {
    sendCmd({ type: 'UnwatchHost' })
    request = null
    watching = false
  }
}

/** Something on screen shows host metrics: watch until the returned function is called. */
export function watchHost(): () => void {
  holders++
  sync()
  let released = false
  return () => {
    if (released) return
    released = true
    holders--
    sync()
  }
}

onServerEvent(({ reply_to, event }) => {
  if (event.type === 'HostSample') {
    if (!watching) return
    useHost.setState((s) => {
      const history = [...s.history, hostPoint(event.data)]
      return { latest: event.data, history: history.length > HOST_HISTORY ? history.slice(-HOST_HISTORY) : history }
    })
    return
  }
  if (event.type === 'Denied' && reply_to !== undefined && reply_to === request) {
    request = null
    watching = false
    useHost.setState({ denied: event.data.reason })
  }
})

onReconnected(() => {
  // The watch lived on the old socket.
  watching = false
  request = null
  sync()
})

if (typeof document !== 'undefined') document.addEventListener('visibilitychange', sync)

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetHostStore()
    return
  }
  if (s.connection === 'open' && prev.connection !== 'open') sync()
})

/** Test hook: forget the watch and its samples (holders stay: they belong to mounted views). */
export function resetHostStore(): void {
  watching = false
  request = null
  useHost.setState({ ...INITIAL })
}

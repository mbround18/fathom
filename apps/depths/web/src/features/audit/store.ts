/**
 * The audit log (US5, T074), read a page at a time over the shared socket.
 *
 * - `setFilter(f)` starts over: `GetAudit{filter}` with no cursor. A reply to an older filter is
 *   dropped, so typing quickly never mixes pages from two filters.
 * - `more()` asks for the page after `next_cursor`.
 * - The log is read-only: there is no command to change it, and this store sends none (FR-036).
 * - A reconnect re-asks for the first page of the current filter.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { AuditEntry, AuditFilter, RequestId } from '@/protocol/generated'

import { cleanFilter } from './model'

export interface AuditState {
  filter: AuditFilter
  entries: AuditEntry[]
  nextCursor: string | null
  /** A page is on its way. */
  loading: boolean
  /** No page has arrived for this filter yet. */
  loaded: boolean
  denied: boolean
  setFilter: (f: AuditFilter) => void
  refresh: () => void
  more: () => void
}

/** Request id → the generation (filter) it was sent for, and whether it was a first page. */
const pending = new Map<RequestId, { gen: number; first: boolean }>()
let gen = 0
let wanted = false

const INITIAL = { filter: {}, entries: [], nextCursor: null, loading: false, loaded: false, denied: false }

export const useAudit = create<AuditState>()((set, get) => ({
  ...INITIAL,

  setFilter: (f) => {
    set({ filter: cleanFilter(f) })
    get().refresh()
  },

  refresh: () => {
    wanted = true
    gen += 1
    set({ entries: [], nextCursor: null, loaded: false, denied: false })
    send(undefined)
  },

  more: () => {
    const { nextCursor, loading } = get()
    if (nextCursor === null || loading) return
    send(nextCursor)
  },
}))

function send(cursor: string | undefined): void {
  const filter = useAudit.getState().filter
  const id = sendCmd({ type: 'GetAudit', data: cursor === undefined ? { filter } : { filter, cursor } })
  if (id === null) {
    useAudit.setState({ loading: false })
    return
  }
  pending.set(id, { gen, first: cursor === undefined })
  useAudit.setState({ loading: true })
}

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  const asked = pending.get(reply_to)
  if (!asked) return
  pending.delete(reply_to)
  if (asked.gen !== gen) return

  if (event.type === 'AuditPage') {
    useAudit.setState((s) => ({
      entries: asked.first ? event.data.entries : [...s.entries, ...event.data.entries],
      nextCursor: event.data.next_cursor,
      loading: false,
      loaded: true,
    }))
  } else if (event.type === 'Denied') {
    useAudit.setState({ denied: true, loading: false, loaded: true })
  }
})

onReconnected(() => {
  pending.clear()
  if (wanted) useAudit.getState().refresh()
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetAuditStore()
    return
  }
  // Asked before the socket was up: ask again now that it is.
  if (s.connection === 'open' && prev.connection !== 'open' && wanted && !useAudit.getState().loaded) {
    useAudit.getState().refresh()
  }
})

/** Test hook. */
export function resetAuditStore(): void {
  pending.clear()
  wanted = false
  gen += 1
  useAudit.setState({ ...INITIAL })
}

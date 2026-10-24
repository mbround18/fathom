/**
 * The stack list and stack details, fed by the shared socket (`app/connection.ts`).
 *
 * - `load()` sends `ListStacks` once; the reply (`Stacks`) and every live `StacksChanged` replace
 *   the snapshot wholesale. Status is only what the server says (FR-009).
 * - `RuntimeUnavailable` keeps the last snapshot and records the reason verbatim (FR-010).
 * - `openDetail(name)` sends `GetStack`; a `Denied` reply to it marks that stack denied. A live
 *   update refreshes an open detail's containers, and re-fetches its files when the hash moved.
 * - A reconnect re-issues `ListStacks` and every open `GetStack`. Leaving the signed-in state
 *   clears everything, so a second person in the same tab never sees the first one's stacks.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { RequestId, StackDetail, StackList } from '@/protocol/generated'

import type { StatusFilter } from './model'

export type DetailEntry =
  | { state: 'loading' }
  | { state: 'ready'; detail: StackDetail }
  | { state: 'denied' }

export interface StacksState {
  list: StackList | null
  /** `RuntimeUnavailable{reason}`, verbatim, until the next snapshot. */
  runtimeError: string | null
  /** The `ListStacks` reply was `Denied`. */
  listDenied: boolean
  query: string
  status: StatusFilter
  details: Record<string, DetailEntry>
  setQuery: (q: string) => void
  setStatus: (s: StatusFilter) => void
  load: () => void
  openDetail: (name: string) => void
  closeDetail: (name: string) => void
}

/** Request ids still waiting for their reply, and what they asked for. */
const pending = new Map<RequestId, { kind: 'list' } | { kind: 'detail'; name: string }>()
/** `load()` was called while there was no socket; send on the next `open`. */
let listWanted = false
let listSent = false
/** `GetStack`s that found no socket; sent on the next `open`. */
const detailsUnsent = new Set<string>()

const INITIAL = {
  list: null,
  runtimeError: null,
  listDenied: false,
  query: '',
  status: 'all' as StatusFilter,
  details: {},
}

export const useStacks = create<StacksState>()((set, get) => ({
  ...INITIAL,

  setQuery: (query) => set({ query }),
  setStatus: (status) => set({ status }),

  load: () => {
    listWanted = true
    if (listSent) return
    sendList()
  },

  openDetail: (name) => {
    const existing = get().details[name]
    if (existing?.state === 'ready') return
    set((s) => ({ details: { ...s.details, [name]: { state: 'loading' } } }))
    sendDetail(name)
  },

  closeDetail: (name) => {
    detailsUnsent.delete(name)
    set((s) => ({ details: Object.fromEntries(Object.entries(s.details).filter(([k]) => k !== name)) }))
  },
}))

function sendList(): void {
  const id = sendCmd({ type: 'ListStacks' })
  if (id === null) return
  listSent = true
  pending.set(id, { kind: 'list' })
}

function sendDetail(name: string): void {
  const id = sendCmd({ type: 'GetStack', data: { name } })
  if (id === null) {
    detailsUnsent.add(name)
    return
  }
  detailsUnsent.delete(name)
  pending.set(id, { kind: 'detail', name })
}

function applySnapshot(list: StackList): void {
  const { details } = useStacks.getState()
  const next: Record<string, DetailEntry> = { ...details }
  for (const [name, entry] of Object.entries(details)) {
    if (entry.state !== 'ready') continue
    const stack = list.stacks.find((s) => s.name === name)
    if (!stack) continue
    if (stack.content_hash !== entry.detail.content_hash) sendDetail(name)
    next[name] = { state: 'ready', detail: { ...entry.detail, stack } }
  }
  useStacks.setState({ list, runtimeError: null, listDenied: false, details: next })
}

onServerEvent(({ reply_to, event }) => {
  const asked = reply_to === undefined ? undefined : pending.get(reply_to)
  if (reply_to !== undefined && asked) pending.delete(reply_to)

  switch (event.type) {
    case 'Stacks':
    case 'StacksChanged':
      applySnapshot(event.data)
      return
    case 'RuntimeUnavailable':
      useStacks.setState((s) => ({
        runtimeError: event.data.reason,
        list: s.list ? { ...s.list, stale: true } : s.list,
      }))
      return
    case 'StackDetail': {
      const name = event.data.stack.name
      useStacks.setState((s) => ({ details: { ...s.details, [name]: { state: 'ready', detail: event.data } } }))
      return
    }
    case 'Denied':
      if (asked?.kind === 'list') useStacks.setState({ listDenied: true })
      else if (asked?.kind === 'detail') {
        const name = asked.name
        useStacks.setState((s) => ({ details: { ...s.details, [name]: { state: 'denied' } } }))
      }
      return
    default:
      return
  }
})

onReconnected(() => {
  pending.clear()
  listSent = false
  if (listWanted) sendList()
  for (const [name, entry] of Object.entries(useStacks.getState().details)) {
    if (entry.state !== 'denied') sendDetail(name)
  }
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetStacksStore()
    return
  }
  // `load()` or `openDetail()` ran before the socket existed: send now that it does.
  if (s.connection === 'open' && prev.connection !== 'open') {
    if (listWanted && !listSent) sendList()
    for (const name of [...detailsUnsent]) sendDetail(name)
  }
})

/** Test hook: forget every request in flight and reset to the signed-out state. */
export function resetStacksStore(): void {
  pending.clear()
  detailsUnsent.clear()
  listWanted = false
  listSent = false
  useStacks.setState({ ...INITIAL })
}

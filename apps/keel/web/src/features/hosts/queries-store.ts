/**
 * The open host's queries (spec 067 US5, T043). `ListHostQueries` (View) reads them newest first,
 * with `query_v1` saying whether the connected runner answers queries (`null`: not connected).
 * `AskHost` (Operate) answers the stored query, still waiting or already settled; the pushed
 * `HostQueryAnswered` settles it in place.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent } from '@/app/connection'
import { loadList, type Refusal } from '@/app/load'
import type { HostQuery, QueryKind } from '@/protocol/wire'

import { paramsFor, upsertQuery } from './queries'

export interface QueriesState {
  host: string | null
  items: HostQuery[] | null
  /** `true`/`false`: the connected runner does / does not speak `query-v1`; `null`: not connected. */
  queryV1: boolean | null
  refusal: Refusal | null
  /** Why the last Ask was refused, until the next one. */
  askRefusal: Refusal | null
  asking: boolean

  load: (host: string) => void
  ask: (kind: QueryKind, value: string) => void
  clear: () => void
}

export const useQueries = create<QueriesState>()((set, get) => ({
  host: null,
  items: null,
  queryV1: null,
  refusal: null,
  askRefusal: null,
  asking: false,

  load: (host) => {
    if (get().host !== host) set({ host, items: null, queryV1: null, refusal: null, askRefusal: null, asking: false })
    else set({ refusal: null })
    loadList(
      { type: 'ListHostQueries', data: { host, limit: null } },
      (e) => (e.type === 'HostQueries' && e.data.host === host ? e.data : undefined),
      {
        loaded: (data) => {
          if (get().host === host) set({ items: data.items, queryV1: data.query_v1, refusal: null })
        },
        failed: (refusal) => {
          if (get().host === host) set({ refusal })
        },
      },
    )
  },

  ask: (kind, value) => {
    const host = get().host
    if (host === null || get().asking) return
    set({ asking: true, askRefusal: null })
    loadList(
      { type: 'AskHost', data: { host, kind, params: paramsFor(kind, value) } },
      (e) => (e.type === 'HostQuery' && e.data.host === host ? e.data : undefined),
      {
        loaded: (q) => {
          if (get().host !== host) return
          set({ asking: false, items: upsertQuery(get().items ?? [], q) })
          // A settled reply says the runner is too old, or gone.
          if (q.outcome === 'no_capability') set({ queryV1: false })
          if (q.outcome === 'disconnected') set({ queryV1: null })
        },
        failed: (askRefusal) => {
          if (get().host === host) set({ asking: false, askRefusal })
        },
      },
    )
  },

  clear: () => {
    set({ host: null, items: null, queryV1: null, refusal: null, askRefusal: null, asking: false })
  },
}))

onServerEvent(({ event }) => {
  if (event.type !== 'HostQueryAnswered') return
  const s = useQueries.getState()
  if (s.host !== event.data.host || s.items === null) return
  useQueries.setState({ items: upsertQuery(s.items, event.data) })
})

onReconnected(() => {
  const { host, load } = useQueries.getState()
  if (host !== null) load(host)
})

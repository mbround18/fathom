/**
 * Join requests (spec 055 T148, contracts/join.md "Accepting and rejecting"): one workspace's
 * requests, narrowed by state (pending by default; the server filters).
 *
 * The pushed `JoinRequested` adds or updates a request without a reload; `JoinRequestDecided` (the
 * answer to a confirmed accept or reject, also pushed to the workspace) moves it out of a pending
 * list. Accepting and rejecting go through the governance confirm dialog (`useGovernance.propose`).
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent } from '@/app/connection'
import { loadList, type Refusal } from '@/app/load'
import type { JoinRequestInfo, JoinRequestState } from '@/protocol/wire'

import { upsertRequest } from './join'

export interface JoinRequestsState {
  workspace: string | null
  /** Narrow to this state; `null` lists every state. */
  state: JoinRequestState | null
  items: JoinRequestInfo[]
  loaded: boolean
  failed: Refusal | null
  load: (workspace: string) => void
  setState: (state: JoinRequestState | null) => void
}

function list(workspace: string, state: JoinRequestState | null): void {
  useJoinRequests.setState({ failed: null })
  loadList(
    { type: 'ListJoinRequests', data: { workspace, state } },
    (event) => (event.type === 'JoinRequests' ? event.data.items : undefined),
    {
      loaded: (items) => {
        const s = useJoinRequests.getState()
        if (s.workspace === workspace && s.state === state) useJoinRequests.setState({ items, loaded: true, failed: null })
      },
      failed: (failed) => {
        if (useJoinRequests.getState().workspace === workspace) useJoinRequests.setState({ failed })
      },
    },
  )
}

export const useJoinRequests = create<JoinRequestsState>()((set, get) => ({
  workspace: null,
  state: 'pending',
  items: [],
  loaded: false,
  failed: null,

  load: (workspace) => {
    if (get().workspace !== workspace) set({ workspace, items: [], loaded: false })
    list(workspace, get().state)
  },
  setState: (state) => {
    set({ state, items: [], loaded: false })
    const workspace = get().workspace
    if (workspace) list(workspace, state)
  },
}))

onServerEvent(({ event }) => {
  if (event.type !== 'JoinRequested' && event.type !== 'JoinRequestDecided') return
  const { request } = event.data
  const s = useJoinRequests.getState()
  if (request.workspace !== s.workspace) return
  useJoinRequests.setState({ items: upsertRequest(s.items, request, s.state) })
})

onReconnected(() => {
  const { workspace, load } = useJoinRequests.getState()
  if (workspace) load(workspace)
})

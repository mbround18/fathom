/**
 * The users list (US5, T074), fed by the shared socket.
 *
 * - `load()` sends `GetUsers`; the reply (`Users`) and every live `UsersChanged` replace the list.
 * - `put` and `remove` send `PutUser` / `RemoveUser`. The server answers with the new list, or
 *   `Denied` (the owner, a role that cannot be assigned), which is kept as `error` until the next
 *   change succeeds.
 * - A reconnect re-issues `GetUsers` if the view asked for it. Leaving the signed-in state clears
 *   everything.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { DenyReason, RequestId, Role, UserList } from '@/protocol/generated'

export interface UsersState {
  list: UserList | null
  /** `GetUsers` itself was refused. */
  denied: boolean
  /** The last change was refused, and why. */
  error: DenyReason | null
  /** Ids with a change in flight. */
  saving: string[]
  load: () => void
  put: (id: string, role: Role, stacks: string[]) => void
  remove: (id: string) => void
  clearError: () => void
}

type Asked = { kind: 'list' } | { kind: 'change'; id: string }

const pending = new Map<RequestId, Asked>()
let wanted = false
let sent = false

const INITIAL = { list: null, denied: false, error: null, saving: [] }

export const useUsers = create<UsersState>()((set) => ({
  ...INITIAL,

  load: () => {
    wanted = true
    if (!sent) sendList()
  },

  put: (id, role, stacks) => {
    change(id, sendCmd({ type: 'PutUser', data: { id, role, stacks } }))
  },

  remove: (id) => {
    change(id, sendCmd({ type: 'RemoveUser', data: { id } }))
  },

  clearError: () => set({ error: null }),
}))

function sendList(): void {
  const id = sendCmd({ type: 'GetUsers' })
  if (id === null) return
  sent = true
  pending.set(id, { kind: 'list' })
}

function change(userId: string, req: RequestId | null): void {
  if (req === null) return
  pending.set(req, { kind: 'change', id: userId })
  useUsers.setState((s) => ({ error: null, saving: [...s.saving, userId] }))
}

function settle(asked: Asked | undefined): void {
  if (asked?.kind !== 'change') return
  useUsers.setState((s) => ({ saving: s.saving.filter((x) => x !== asked.id) }))
}

onServerEvent(({ reply_to, event }) => {
  const asked = reply_to === undefined ? undefined : pending.get(reply_to)
  if (reply_to !== undefined && asked) pending.delete(reply_to)

  switch (event.type) {
    case 'Users':
    case 'UsersChanged':
      settle(asked)
      useUsers.setState({ list: event.data, denied: false })
      return
    case 'Denied':
      if (!asked) return
      settle(asked)
      if (asked.kind === 'list') useUsers.setState({ denied: true })
      else useUsers.setState({ error: event.data.reason })
      return
    default:
      return
  }
})

onReconnected(() => {
  pending.clear()
  sent = false
  useUsers.setState({ saving: [] })
  if (wanted) sendList()
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetUsersStore()
    return
  }
  if (s.connection === 'open' && prev.connection !== 'open' && wanted && !sent) sendList()
})

/** Test hook: forget every request in flight. */
export function resetUsersStore(): void {
  pending.clear()
  wanted = false
  sent = false
  useUsers.setState({ ...INITIAL })
}

/**
 * The users list (US5, T074), fed by the shared socket.
 *
 * - `load()` sends `GetUsers`; the reply (`Users`) and every live `UsersChanged` replace the list.
 * - `put` and `remove` send `PutUser` / `RemoveUser`. The server answers with the new list, or
 *   `Denied` (the owner, a role that cannot be assigned), which is kept as `error` until the next
 *   change succeeds.
 * - A change that needs a fresh passkey check (054: `StepUpRequired`) opens the passkey dialog
 *   (`requestFreshCheck`) and, once the session is fresh, sends the same change again. Cancelled,
 *   or without a passkey, it is kept as `stepUp` the way `error` is.
 * - A reconnect re-issues `GetUsers` if the view asked for it. Leaving the signed-in state clears
 *   everything.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { StepUpRefusal } from '@/features/passkeys/messages'
import type { Cmd, DenyReason, RequestId, Role, UserList } from '@/protocol/generated'
import { OFFLINE, refusalOf, type Refusal } from '@/protocol/refusal'

export interface UsersState {
  list: UserList | null
  /** Why `GetUsers` itself failed (spec 065 T021). */
  failed: Refusal | null
  /** The last change was refused, and why. */
  error: DenyReason | null
  /** The last change needed a passkey check that did not happen. */
  stepUp: StepUpRefusal | null
  /** Ids with a change in flight. */
  saving: string[]
  load: () => void
  /** Ask for the list again after it failed. */
  retry: () => void
  put: (id: string, role: Role, stacks: string[]) => void
  remove: (id: string) => void
  clearError: () => void
}

type Asked = { kind: 'list' } | { kind: 'change'; id: string; cmd: Cmd }

const pending = new Map<RequestId, Asked>()
let wanted = false
let sent = false

const INITIAL = { list: null, failed: null, error: null, stepUp: null, saving: [] }

export const useUsers = create<UsersState>()((set) => ({
  ...INITIAL,

  load: () => {
    wanted = true
    if (!sent) sendList()
  },

  retry: () => {
    wanted = true
    sent = false
    set({ failed: null })
    if (!sendList() && useSession.getState().connection === 'closed') set({ failed: OFFLINE })
  },

  put: (id, role, stacks) => {
    change(id, { type: 'PutUser', data: { id, role, stacks } })
  },

  remove: (id) => {
    change(id, { type: 'RemoveUser', data: { id } })
  },

  clearError: () => set({ error: null, stepUp: null }),
}))

/** Asks for the list; whether it went out (no socket yet: not). */
function sendList(): boolean {
  const id = sendCmd({ type: 'GetUsers' })
  if (id === null) return false
  sent = true
  pending.set(id, { kind: 'list' })
  return true
}

function change(userId: string, cmd: Cmd): void {
  const req = sendCmd(cmd)
  if (req === null) return
  pending.set(req, { kind: 'change', id: userId, cmd })
  useUsers.setState((s) => ({ error: null, stepUp: null, saving: [...s.saving, userId] }))
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
      useUsers.setState({ list: event.data, failed: null })
      return
    case 'Denied':
      if (!asked) return
      settle(asked)
      if (asked.kind === 'list') useUsers.setState({ failed: refusalOf(event) })
      else useUsers.setState({ error: event.data.reason })
      return
    case 'Invalid':
      if (asked?.kind === 'list') useUsers.setState({ failed: refusalOf(event) })
      return
    case 'PasskeyEnrolmentRequired':
      if (asked?.kind !== 'change') return
      settle(asked)
      useUsers.setState({ stepUp: { kind: 'enrol' } })
      return
    case 'StepUpRequired': {
      if (asked?.kind !== 'change') return
      const request = event.data.request
      void import('@/features/passkeys/store').then(async ({ usePasskeys }) => {
        const fresh = await usePasskeys.getState().requestFreshCheck('Changing a user', request)
        settle(asked)
        if (fresh) change(asked.id, asked.cmd)
        else useUsers.setState({ stepUp: { kind: 'not_confirmed' } })
      })
      return
    }
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

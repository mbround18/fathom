/**
 * The Updates badge (spec 062 US1): how many stacks have an update. The server sends
 * `UpdatesBadge` to admins on sign-in and whenever the count changes; nobody else gets one.
 *
 * Imported by the shell, so it must stay small: the page itself is a lazy chunk.
 */
import { create } from 'zustand'

import { onServerEvent } from '@/app/connection'
import { useSession } from '@/app/session-store'

export const useUpdatesBadge = create<{ count: number | null }>()(() => ({ count: null }))

onServerEvent(({ event }) => {
  if (event.type === 'UpdatesBadge') useUpdatesBadge.setState({ count: event.data.stacks_with_updates })
  else if (event.type === 'UpdatesChanged' || event.type === 'Updates') {
    const stacks = new Set(event.data.services.filter((s) => s.state === 'available').map((s) => s.stack))
    useUpdatesBadge.setState({ count: stacks.size })
  }
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') useUpdatesBadge.setState({ count: null })
})

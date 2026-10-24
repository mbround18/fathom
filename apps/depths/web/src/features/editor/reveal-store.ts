/**
 * Revealed `.env` text (FR-021), fed by the shared socket. `reveal(name)` sends `RevealEnv`; the
 * server audits it and answers `EnvRevealed` with the real text, or `Denied`. Only the editor
 * reads it, and only after the person asked. Forgotten when the editor closes (`forget`), on a
 * reconnect, and on sign-out.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { DenyReason, RequestId } from '@/protocol/generated'

export type Reveal =
  | { state: 'asking' }
  | { state: 'ready'; text: string }
  | { state: 'denied'; reason: DenyReason | null }

interface RevealState {
  env: Record<string, Reveal>
  reveal: (name: string) => void
  forget: (name: string) => void
}

const pending = new Map<RequestId, string>()

const put = (name: string, r: Reveal): void => useReveal.setState((s) => ({ env: { ...s.env, [name]: r } }))

export const useReveal = create<RevealState>()(() => ({
  env: {},
  reveal: (name) => {
    const id = sendCmd({ type: 'RevealEnv', data: { name } })
    if (id === null) {
      put(name, { state: 'denied', reason: null })
      return
    }
    pending.set(id, name)
    put(name, { state: 'asking' })
  },
  forget: (name) => {
    useReveal.setState((s) => ({ env: Object.fromEntries(Object.entries(s.env).filter(([k]) => k !== name)) }))
  },
}))

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  const name = pending.get(reply_to)
  if (name === undefined) return
  pending.delete(reply_to)
  if (event.type === 'EnvRevealed' && event.data.name === name) put(name, { state: 'ready', text: event.data.env_text })
  else if (event.type === 'Denied') put(name, { state: 'denied', reason: event.data.reason })
})

onReconnected(() => {
  // The reply went with the old socket; ask again rather than show a spinner forever.
  for (const name of pending.values()) put(name, { state: 'denied', reason: null })
  pending.clear()
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') resetRevealStore()
})

export function resetRevealStore(): void {
  pending.clear()
  useReveal.setState({ env: {} })
}

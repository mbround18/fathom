/**
 * Revealed `.env` text (FR-021), fed by the shared socket. `reveal(name)` sends `RevealEnv`; the
 * server audits it and answers `EnvRevealed` with the real text, or `Denied`. Revealing is
 * admin-only (the server refuses everyone else; `useCanReveal` hides the button to match).
 *
 * Revealed text is forgotten (masked again) when the view that asked for it closes (`forget`),
 * `REVEAL_TTL_MS` after it arrived, and on sign-out. A reply still pending when the socket drops
 * is dropped too.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { DenyReason, RequestId } from '@/protocol/generated'

/** How long revealed values stay on screen before they are masked again. */
export const REVEAL_TTL_MS = 5 * 60_000

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
const expiry = new Map<string, ReturnType<typeof setTimeout>>()

function clearExpiry(name: string): void {
  const t = expiry.get(name)
  if (t !== undefined) clearTimeout(t)
  expiry.delete(name)
}

const put = (name: string, r: Reveal): void => useReveal.setState((s) => ({ env: { ...s.env, [name]: r } }))

export const useReveal = create<RevealState>()(() => ({
  env: {},
  reveal: (name) => {
    clearExpiry(name)
    const id = sendCmd({ type: 'RevealEnv', data: { name } })
    if (id === null) {
      put(name, { state: 'denied', reason: null })
      return
    }
    pending.set(id, name)
    put(name, { state: 'asking' })
  },
  forget: (name) => {
    clearExpiry(name)
    for (const [id, n] of pending) if (n === name) pending.delete(id)
    useReveal.setState((s) => ({ env: Object.fromEntries(Object.entries(s.env).filter(([k]) => k !== name)) }))
  },
}))

/** Whether the person may reveal `.env` values: admins only, as the server enforces. */
export function useCanReveal(): boolean {
  return useSession((s) => s.status === 'ready' && s.me?.role === 'admin')
}

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  const name = pending.get(reply_to)
  if (name === undefined) return
  pending.delete(reply_to)
  if (event.type === 'EnvRevealed' && event.data.name === name) {
    put(name, { state: 'ready', text: event.data.env_text })
    clearExpiry(name)
    expiry.set(
      name,
      setTimeout(() => useReveal.getState().forget(name), REVEAL_TTL_MS),
    )
  } else if (event.type === 'Denied') put(name, { state: 'denied', reason: event.data.reason })
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
  for (const t of expiry.values()) clearTimeout(t)
  expiry.clear()
  useReveal.setState({ env: {} })
}

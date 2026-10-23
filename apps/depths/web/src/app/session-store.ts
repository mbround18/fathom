/**
 * Who is signed in, and whether the shell may show anything.
 *
 * Bootstrapped from `GET /api/v1/me`. The CSRF token lives here, in memory only: it is sent in
 * `Hello` and as `X-CSRF-Token`, and never written to storage or a URL.
 *
 * - `signed_out`: a 401 from `/me`, or the socket closed 4401. Shows the sign-in page.
 * - `no_access`: role `pending`, a `user` with no stacks (FR-031), or the socket closed 4403.
 *   Shows the no-access page, which says nothing about the host.
 * - `too_many`: the socket closed 4429 (too many tabs on this session).
 * - `ready`: the routes render and the socket is up (or reconnecting).
 */
import { create } from 'zustand'

import { fetchMe, mutate, type FetchLike } from '@/protocol/http'
import type { Me, Provider } from '@/protocol/generated'
import type { ConnectionStatus, TerminalReason } from '@/protocol/transport'

import { connect, disconnect, onServerEvent } from './connection'

export type SessionStatus = 'loading' | 'signed_out' | 'no_access' | 'too_many' | 'ready' | 'error'

export interface SessionState {
  status: SessionStatus
  me: Me | null
  /** Sign-in providers the server's 401 named. */
  providers: Provider[]
  connection: ConnectionStatus
  bootstrap: (fetchFn?: FetchLike) => Promise<void>
  signOut: () => Promise<void>
}

/** FR-031: nothing to show this person. */
export function hasNoAccess(me: Me): boolean {
  return me.role === 'pending' || (me.role === 'user' && me.stacks.length === 0)
}

export const useSession = create<SessionState>()((set, get) => {
  const refreshProviders = async (): Promise<void> => {
    const r = await fetchMe()
    if (r.kind === 'unauthenticated') set({ providers: r.providers })
  }

  const onTerminal = (reason: TerminalReason): void => {
    if (reason === 'unauthenticated') {
      set({ status: 'signed_out', me: null })
      void refreshProviders()
    } else if (reason === 'forbidden') {
      set({ status: 'no_access' })
    } else {
      set({ status: 'too_many' })
    }
  }

  // The server warns 60 s before the session ends; refresh it over HTTP.
  onServerEvent(({ event }) => {
    if (event.type !== 'SessionExpiring') return
    const csrf = get().me?.csrf_token
    if (!csrf) return
    void mutate('/auth/refresh', csrf).then(
      (res) => {
        if (res.status === 401) onTerminal('unauthenticated')
      },
      () => undefined,
    )
  })

  return {
    status: 'loading',
    me: null,
    providers: [],
    connection: 'closed',

    bootstrap: async (fetchFn) => {
      set({ status: 'loading' })
      const r = await fetchMe(fetchFn)
      if (r.kind === 'unauthenticated') {
        disconnect()
        set({ status: 'signed_out', me: null, providers: r.providers })
        return
      }
      if (r.kind === 'error') {
        set({ status: 'error' })
        return
      }
      const me = r.me
      if (hasNoAccess(me)) {
        disconnect()
        set({ status: 'no_access', me })
        return
      }
      set({ status: 'ready', me })
      connect(me.csrf_token, {
        onStatus: (connection) => set({ connection }),
        onTerminal,
      })
    },

    signOut: async () => {
      const csrf = get().me?.csrf_token
      disconnect()
      let endSessionUrl: string | null = null
      if (csrf) {
        try {
          const res = await mutate('/auth/logout', csrf)
          const body: unknown = await res.json().catch(() => null)
          if (body && typeof body === 'object' && 'endSessionUrl' in body) {
            const u = body.endSessionUrl
            if (typeof u === 'string' && /^https?:\/\//.test(u)) endSessionUrl = u
          }
        } catch {
          /* The session is gone either way from this tab's view. */
        }
      }
      set({ status: 'signed_out', me: null })
      if (endSessionUrl) globalThis.location.assign(endSessionUrl)
      else void refreshProviders()
    },
  }
})

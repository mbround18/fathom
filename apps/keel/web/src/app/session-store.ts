/**
 * Who is signed in, and whether the shell may show anything.
 *
 * Keel has no `/api/v1/me`. The page reads the session's CSRF token from its readable cookie
 * (protocol/http.ts); without one there is no session and nothing is opened. With one, the socket
 * says `Hello` with it and asks `GetMe`; `Me` makes the shell ready. The token lives here, in
 * memory only, for `Hello` and `X-CSRF-Token`.
 *
 * - `signed_out`: no CSRF cookie, or the socket closed 4401/4403. Shows the sign-in page.
 *   `notice` says why when it happened under a signed-in page: `expired`, or `revoked` after a
 *   `Revoked` push (the session's access changed, R11).
 * - `outdated`: the socket closed 4400 (this page speaks another protocol version than the server).
 * - `error`: `GetMe` failed.
 * - `ready`: `Me` arrived; the routes render and the socket is up (or reconnecting).
 */
import { create } from 'zustand'

import { endSessionUrl, mutate, readCsrf, type FetchLike } from '@/protocol/http'
import type { ConnectionStatus, TerminalReason, TransportEnv } from '@/protocol/transport'
import type { Me } from '@/protocol/wire'

import { connect, disconnect, onReconnected, onServerEvent, sendCmd } from './connection'
import { takeReturnTo } from './return-to'
import { navigate } from './router'

export type SessionStatus = 'loading' | 'signed_out' | 'outdated' | 'error' | 'ready'
export type SignedOutNotice = 'expired' | 'revoked' | null

export interface BootstrapOptions {
  /** `document.cookie` by default. */
  cookies?: string
  transportEnv?: TransportEnv
}

export interface SessionState {
  status: SessionStatus
  me: Me | null
  connection: ConnectionStatus
  notice: SignedOutNotice
  bootstrap: (opts?: BootstrapOptions) => void
  signOut: (fetchFn?: FetchLike) => Promise<void>
}

/** The session's CSRF token, in memory only. */
let csrf: string | null = null

export const useSession = create<SessionState>()((set, get) => {
  const signedOut = (notice: SignedOutNotice): void => {
    csrf = null
    set({ status: 'signed_out', me: null, notice })
  }

  const onTerminal = (reason: TerminalReason): void => {
    if (reason === 'outdated') {
      set({ status: 'outdated', me: null })
    } else if (reason === 'revoked') {
      signedOut('revoked')
    } else {
      signedOut(get().status === 'ready' ? 'expired' : null)
    }
  }

  onServerEvent(({ event }) => {
    const { status, me } = get()
    if (event.type === 'Me') {
      if (status !== 'loading' && status !== 'ready') return
      set({ status: 'ready', me: event.data, notice: null })
      if (status === 'loading') {
        const back = takeReturnTo()
        if (back) navigate(back, { replace: true })
      }
    } else if (event.type === 'Verification') {
      if (me) set({ me: { ...me, verification: event.data } })
    } else if (event.type === 'PasskeyRegistered') {
      if (me) set({ me: { ...me, verification: { ...me.verification, enrolled: true, recovering: false } } })
    } else if (event.type === 'OpError' && status === 'loading') {
      set({ status: 'error' })
    }
  })

  // Anything may have changed while the socket was down (grants, verification).
  onReconnected(() => {
    if (get().status === 'ready') sendCmd({ type: 'GetMe' })
  })

  return {
    status: 'loading',
    me: null,
    connection: 'closed',
    notice: null,

    bootstrap: (opts = {}) => {
      disconnect()
      const token = readCsrf(opts.cookies ?? globalThis.document.cookie)
      if (!token) {
        signedOut(null)
        return
      }
      csrf = token
      set({ status: 'loading', me: null, notice: null })
      connect(token, { onStatus: (connection) => set({ connection }), onTerminal }, opts.transportEnv)
      sendCmd({ type: 'GetMe' })
    },

    signOut: async (fetchFn) => {
      const token = csrf
      disconnect()
      let end: string | null = null
      if (token) {
        try {
          end = await endSessionUrl(await mutate('/auth/logout', token, {}, fetchFn))
        } catch {
          /* The session is gone either way from this tab's view. */
        }
      }
      signedOut(null)
      if (end) globalThis.location.assign(end)
    },
  }
})

import { disconnect } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { TransportEnv } from '@/protocol/transport'
import type { Me } from '@/protocol/wire'

import { FakeSocket } from './fake-socket'

export const ME: Me = {
  id: 'u-1',
  display_name: 'Ada Lovelace',
  email: 'ada@example.com',
  instance_role: 'admin',
  grants: [],
  verification: { verified: false, at: null, window_secs: 900, enrolled: false, required: false, recovering: false },
}

/** Reconnect timers the test fires by hand. */
export const timers: (() => void)[] = []

export const env: TransportEnv = {
  origin: 'http://localhost',
  factory: (u) => new FakeSocket(u),
  setTimer: (fn) => timers.push(fn),
  clearTimer: () => undefined,
}

export interface Frame {
  id?: number
  cmd?: { type: string; data?: unknown }
  csrf_token?: string
}

export function frames(s: FakeSocket): Frame[] {
  return s.frames() as Frame[]
}

/** The last command of `type` the page sent on `s`. */
export function lastCmd(s: FakeSocket, type: string): Frame | undefined {
  return frames(s).findLast((f) => f.cmd?.type === type)
}

/** Signs in: cookie, socket open, `Me` answering `GetMe`. */
export function signIn(me: Me = ME): FakeSocket {
  useSession.getState().bootstrap({ cookies: 'keel_csrf=tok-1', transportEnv: env })
  const s = FakeSocket.last()
  s.open()
  s.serverSend({ reply_to: lastCmd(s, 'GetMe')?.id, event: { type: 'Me', data: me } })
  return s
}

export function resetSession(): void {
  disconnect()
  FakeSocket.reset()
  timers.length = 0
  sessionStorage.clear()
  history.replaceState(null, '', '/')
  useSession.setState({ status: 'loading', me: null, notice: null, connection: 'closed' })
}

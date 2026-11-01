/**
 * The one path every list read takes (spec 063 follow-up): send the command, wait for its answer,
 * and say why when it fails, instead of loading forever.
 *
 * `loadList(cmd, pick, on)` sends `cmd`. The first reply `pick` accepts is the list (`on.loaded`);
 * a `Denied` or `OpError` reply is a refusal, kept as the server sent it (`on.failed`). With no
 * socket at all (signed out, no access) it fails straight away as `OFFLINE`. Other replies, and
 * pushes, are left to the store's own `onServerEvent`. Stores keep the failure on that list's
 * state and the page shows it with `LoadFailed`, whose Retry calls the store's load again.
 */
import type { Cmd, Event, RequestId } from '@/protocol/wire'

import { onServerEvent, sendCmd } from './connection'

/** A refusal as the server sent it, or `OFFLINE`. */
export type Refusal =
  | { kind: 'denied'; action: string; reason: string }
  | { kind: 'error'; code: string; message: string }

/** No socket to send on: not the server's answer, so it is shown in plain words. */
export const OFFLINE: Refusal = {
  kind: 'error',
  code: 'offline',
  message: 'Keel is not connected. Wait for it to reconnect, then try again.',
}

/** A failure on Keel's side of the connection rather than a refusal from the server. */
export function isTransportFailure(r: Refusal): boolean {
  return r === OFFLINE || (r.kind === 'error' && r.code === 'offline')
}

/** `Denied` and `OpError` as a `Refusal`; anything else is not one. */
export function refusalOf(event: Event): Refusal | null {
  if (event.type === 'Denied') return { kind: 'denied', action: event.data.action, reason: event.data.reason }
  if (event.type === 'OpError') return { kind: 'error', code: event.data.code, message: event.data.message }
  return null
}

export interface ListHandlers<T> {
  loaded: (value: T) => void
  failed: (refusal: Refusal) => void
}

const pending = new Map<RequestId, (event: Event) => boolean>()

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  const handle = pending.get(reply_to)
  if (handle?.(event)) pending.delete(reply_to)
})

/** Sends a list read; its answer lands in `on.loaded`, or its refusal in `on.failed`. */
export function loadList<T>(cmd: Cmd, pick: (event: Event) => T | undefined, on: ListHandlers<T>): void {
  const id = sendCmd(cmd)
  if (id === null) {
    on.failed(OFFLINE)
    return
  }
  pending.set(id, (event) => {
    const value = pick(event)
    if (value !== undefined) {
      on.loaded(value)
      return true
    }
    const refusal = refusalOf(event)
    if (refusal) on.failed(refusal)
    return refusal !== null
  })
}

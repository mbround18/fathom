/**
 * The one socket, shared by every feature store.
 *
 * Feature stores (`features/<name>/store.ts`) never hold the transport. They call `sendCmd` and
 * subscribe with `onServerEvent`, filtering on `event.type` and, for replies, on `reply_to`.
 * On a reconnect `onReconnected` fires so a store can re-issue its watches (`GetMe`,
 * `SubscribeRun` with `from_seq`, ...).
 */
import type { Cmd, EventEnvelope, RequestId } from '@/protocol/wire'
import {
  browserTransportEnv,
  Transport,
  type ConnectionStatus,
  type TerminalReason,
  type TransportEnv,
} from '@/protocol/transport'

type EventListener = (envelope: EventEnvelope) => void

let transport: Transport | null = null
const eventListeners = new Set<EventListener>()
const reconnectListeners = new Set<() => void>()

export interface ConnectCallbacks {
  onStatus: (status: ConnectionStatus) => void
  onTerminal: (reason: TerminalReason) => void
}

export function connect(
  csrfToken: string,
  callbacks: ConnectCallbacks,
  env: TransportEnv = browserTransportEnv(),
): void {
  disconnect()
  transport = new Transport(env, csrfToken, {
    onEvent: (envelope) => {
      for (const l of eventListeners) l(envelope)
    },
    onStatus: (status, { reconnected }) => {
      callbacks.onStatus(status)
      if (reconnected) for (const l of reconnectListeners) l()
    },
    onTerminal: (reason) => {
      transport = null
      callbacks.onTerminal(reason)
    },
  })
}

export function disconnect(): void {
  const t = transport
  transport = null
  t?.close()
}

/** Send a command; `null` when there is no socket (signed out, no access). */
export function sendCmd(cmd: Cmd): RequestId | null {
  return transport ? transport.send(cmd) : null
}

export function onServerEvent(listener: EventListener): () => void {
  eventListeners.add(listener)
  return () => eventListeners.delete(listener)
}

export function onReconnected(listener: () => void): () => void {
  reconnectListeners.add(listener)
  return () => reconnectListeners.delete(listener)
}

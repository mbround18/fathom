/**
 * The webview's only link to the backend (contracts/ipc.md): two calls, both typed by the
 * generated protocol. `VITE_TRANSPORT=fake` (`pnpm dev:fake`, Playwright) swaps in the scripted
 * in-page backend, `VITE_TRANSPORT=ws` (the live e2e suite) the dev WebSocket bridge; anything
 * else uses Tauri IPC.
 */
import type { BackendCmd, BridgeError, EventBatch, SubscriptionInfo } from './types'

export interface Transport {
  /**
   * Resolves to the command's `RequestId` (monotonic from 1). Every event the command
   * produces carries `correlation: { type: 'Request', data: id }`, ending with exactly one
   * `CommandFinished`. Rejects only with a `BridgeError` (backend gone).
   */
  send(cmd: BackendCmd): Promise<number>
  /** Attach the single event subscriber, replacing any previous one. */
  subscribe(onBatch: (batch: EventBatch) => void): Promise<SubscriptionInfo>
  /** Transports that can notice the backend going away unprompted (ws) report it here. */
  onGone?(listener: (err: BridgeError) => void): void
}

export type TransportKind = 'tauri' | 'fake' | 'ws' | 'hosted'

export function transportKind(): TransportKind {
  const kind = import.meta.env.VITE_TRANSPORT
  return kind === 'fake' || kind === 'ws' || kind === 'hosted' ? kind : 'tauri'
}

let current: Promise<Transport> | undefined

/**
 * The app-wide transport. Both implementations are loaded lazily so a Tauri build never
 * bundles the fake backend or its fixtures (Vite folds `VITE_TRANSPORT` at build time).
 */
export function getTransport(): Promise<Transport> {
  current ??=
    import.meta.env.VITE_TRANSPORT === 'fake'
      ? import('./fake-transport').then((m) => m.installFakeTransport())
      : import.meta.env.VITE_TRANSPORT === 'ws'
        ? import('./ws-transport').then((m) => m.createWsTransport())
        : import.meta.env.VITE_TRANSPORT === 'hosted'
          ? import('./hosted-transport').then((m) => m.createHostedTransport())
          : import('./tauri-transport').then((m) => new m.TauriTransport())
  return current
}

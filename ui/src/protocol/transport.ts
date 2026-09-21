/**
 * The webview's only link to the backend (contracts/ipc.md): two calls, both typed by the
 * generated protocol. `VITE_TRANSPORT=fake` (`pnpm dev:fake`, Playwright) swaps in the scripted
 * in-page backend; anything else uses Tauri IPC.
 */
import type { BackendCmd, EventBatch, SubscriptionInfo } from './types'

export interface Transport {
  /** Fire-and-forget. Rejects only if the backend is gone (`BridgeError`). */
  send(cmd: BackendCmd): Promise<void>
  /** Attach the single event subscriber, replacing any previous one. */
  subscribe(onBatch: (batch: EventBatch) => void): Promise<SubscriptionInfo>
}

export type TransportKind = 'tauri' | 'fake'

export function transportKind(): TransportKind {
  return import.meta.env.VITE_TRANSPORT === 'fake' ? 'fake' : 'tauri'
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
      : import('./tauri-transport').then((m) => new m.TauriTransport())
  return current
}

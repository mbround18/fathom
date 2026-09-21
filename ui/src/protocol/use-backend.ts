/**
 * Connects the store to the backend (data-model.md §4, connection):
 *
 *   starting ──subscribe ok──▶ ready ──send rejected──▶ backend-gone (fatal; retry = restart)
 *
 * On mount it subscribes, routes every batch into `ingest`, installs `store.send`, and sends the
 * bootstrap commands (`LoadUiConfig`, `RefreshContexts`, `Initialize`). The connection is
 * app-lifetime: StrictMode's double mount and later remounts reuse the first one.
 */
import { useEffect } from 'react'
import type { StoreApi } from 'zustand'

import { useStore, type StoreState } from '@/store'
import { ingest } from '@/store/ingest'

import { BOOTSTRAP_CMDS } from './commands'
import { getTransport, type Transport } from './transport'
import type { BackendCmd } from './types'

/** Text for a rejected `send_cmd`/`subscribe_events`, kept verbatim where possible. */
export function describeBridgeError(err: unknown): string {
  if (typeof err === 'object' && err !== null && 'type' in err && typeof err.type === 'string') {
    return err.type === 'BackendGone'
      ? 'The Fathom backend has stopped (BackendGone). Restart the app to reconnect.'
      : `Bridge error: ${err.type}`
  }
  if (err instanceof Error) return err.message
  return String(err)
}

/**
 * Wire `transport` to `store`. Resolves once subscribed and bootstrapped; never rejects (a
 * failure becomes `conn.status = 'backend-gone'`).
 */
export async function connectBackend(
  transport: Transport,
  store: StoreApi<StoreState> = useStore,
): Promise<void> {
  const fail = (err: unknown): void => {
    const { conn } = store.getState()
    if (conn.status === 'backend-gone') return
    store.setState({ conn: { ...conn, status: 'backend-gone', fatal: describeBridgeError(err) } })
  }
  const send = (cmd: BackendCmd): void => {
    if (store.getState().conn.status === 'backend-gone') return
    transport.send(cmd).catch(fail)
  }

  store.setState({ send, conn: { ...store.getState().conn, status: 'starting' } })
  try {
    await transport.subscribe((batch) => {
      ingest(batch, store)
    })
  } catch (err) {
    fail(err)
    return
  }
  // A version-mismatch batch may already have made the connection fatal.
  const { conn } = store.getState()
  if (conn.status === 'starting') store.setState({ conn: { ...conn, status: 'ready' } })
  for (const cmd of BOOTSTRAP_CMDS) send(cmd)
}

let connection: Promise<void> | undefined

/** Mount once near the app root. */
export function useBackend(): void {
  useEffect(() => {
    connection ??= getTransport().then(
      (transport) => connectBackend(transport),
      (err: unknown) => {
        const { conn } = useStore.getState()
        useStore.setState({
          conn: { ...conn, status: 'backend-gone', fatal: describeBridgeError(err) },
        })
      },
    )
  }, [])
}

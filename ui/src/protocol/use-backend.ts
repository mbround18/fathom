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
import { toast } from 'sonner'
import type { StoreApi } from 'zustand'

import { useStore, type StoreState } from '@/store'
import { applySubscriptionInfo, isHosted } from '@/store/handlers/hosted'
import { ingest } from '@/store/ingest'

import { BOOTSTRAP_CMDS } from './commands'
import { getTransport, type Transport } from './transport'
import type { BackendCmd } from './types'

/** Text for a rejected `send_cmd`/`subscribe_events`, kept verbatim where possible. */
export function describeBridgeError(err: unknown): string {
  if (isForbidden(err)) return `Refused by the dev bridge write guard (Forbidden): ${err.data}`
  if (typeof err === 'object' && err !== null && 'type' in err && typeof err.type === 'string') {
    return err.type === 'BackendGone'
      ? 'The Fathom backend has stopped (BackendGone). Restart the app to reconnect.'
      : `Bridge error: ${err.type}`
  }
  if (err instanceof Error) return err.message
  return String(err)
}

/**
 * The dev bridge's write guard refused one command (`BridgeError::Forbidden`). Unlike every
 * other rejection this is not fatal: the backend is still there.
 */
function isForbidden(err: unknown): err is { type: 'Forbidden'; data: string } {
  return (
    typeof err === 'object' &&
    err !== null &&
    'type' in err &&
    err.type === 'Forbidden' &&
    'data' in err &&
    typeof err.data === 'string'
  )
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
    transport.send(cmd).catch((err: unknown) => {
      if (isForbidden(err)) toast.error(describeBridgeError(err))
      else fail(err)
    })
  }

  const request = async (cmd: BackendCmd): Promise<number> => {
    if (store.getState().conn.status === 'backend-gone') {
      throw new Error(store.getState().conn.fatal ?? 'The Fathom backend has stopped.')
    }
    try {
      // Resolves to the command's `RequestId`; its result arrives as `CommandFinished`.
      return await transport.send(cmd)
    } catch (err) {
      if (!isForbidden(err)) fail(err)
      throw new Error(describeBridgeError(err), { cause: err })
    }
  }

  store.setState({ send, request, conn: { ...store.getState().conn, status: 'starting' } })
  transport.onGone?.(fail)
  try {
    const info = await transport.subscribe((batch) => {
      ingest(batch, store)
    })
    // Idempotent: a duplicate SubscriptionInfo after a reconnect only refreshes these fields
    // and keeps any terminal state (session ended, limit reached) already recorded.
    store.setState((s) => ({ hosted: applySubscriptionInfo(s.hosted, info) }))
  } catch (err) {
    fail(err)
    return
  }
  // A version-mismatch batch may already have made the connection fatal.
  const { conn } = store.getState()
  if (conn.status === 'starting') store.setState({ conn: { ...conn, status: 'ready' } })
  for (const cmd of BOOTSTRAP_CMDS) send(cmd)
  // 048-docker-management (FR-001/FR-002): ask once per session whether this computer has a
  // container runtime. Here rather than in a component, because whether the Containers group
  // exists must not depend on which pane happens to be mounted — at phone width the navigator
  // is not mounted until the operator opens it, and the answer would never be asked for.
  // Never polled (Principle V), and not asked in hosted mode: a server pod has no local daemon
  // and every command in the family is denied there, so asking earns a refusal nobody wanted.
  if (!isHosted(store.getState().hosted)) send({ type: 'RefreshContainerRuntimePresence' })
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

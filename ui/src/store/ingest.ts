/**
 * The event reducer (data-model.md §3): one `set()` per `EventBatch`, rules applied in order.
 *
 * 1. Drop envelopes whose `route.context` is set and is not the active context (cluster events,
 *    nodes, lists, reads, and context-bound System results such as the Flux overview). Envelopes
 *    with a `null` context (global System events: notifications, contexts, UI config) are kept.
 * 2. Drop `ResourceList`/`ResourceRead` envelopes whose `Request` correlation is older than
 *    the latest request already applied to the same list/detail key (FR-016).
 * 3. List replacement reuses unchanged row objects (handlers/list.ts).
 * 4. Stream chunks append to a bounded ring buffer (handlers/stream.ts).
 * 5. Action results resolve `pending[request_id]` with a toast (handlers/system.ts).
 *
 * A batch with any envelope whose `version` is not 1 is a protocol mismatch: the whole batch
 * is ignored and the connection enters the fatal `backend-gone` state.
 */
import type { StoreApi } from 'zustand'

import { PROTOCOL_VERSION, type BackendEventEnvelope, type EventBatch } from '@/protocol/types'

import { Draft } from './draft'
import { handleList } from './handlers/list'
import { handleRead, readKeyOf } from './handlers/read'
import { handleStream } from './handlers/stream'
import { handleSystem } from './handlers/system'
import { useStore, type StoreData, type StoreState } from './index'
import { listKeyOf } from './keys'

/** Apply one batch to the store with exactly one `set()`, then run its side effects (toasts). */
export function ingest(batch: EventBatch, store: StoreApi<StoreState> = useStore): void {
  let draft: Draft | undefined
  store.setState((s) => {
    draft = reduceBatch(s, batch)
    return draft.patch
  })
  for (const effect of draft?.effects ?? []) effect()
}

/** Pure(ish) reducer: returns the draft holding the patch and deferred effects. */
export function reduceBatch(state: StoreData, batch: EventBatch): Draft {
  const d = new Draft(state)

  if (batch.dropped > 0) {
    const conn = d.edit('conn')
    conn.dropped += batch.dropped
  }

  const bad = batch.envelopes.find((e) => e.version !== PROTOCOL_VERSION)
  if (bad) {
    const conn = d.edit('conn')
    conn.status = 'backend-gone'
    conn.fatal = `Unsupported protocol version ${bad.version} (this UI speaks version ${PROTOCOL_VERSION}). Update Fathom.`
    return d
  }

  for (const env of batch.envelopes) applyEnvelope(d, env)
  return d
}

function applyEnvelope(d: Draft, env: BackendEventEnvelope): void {
  const event = env.event
  // Rule 1: results for another context are stale (the operator switched away).
  if (env.route.context !== null && env.route.context !== d.read('session').activeContext) return

  if (event.type === 'System') {
    handleSystem(d, env, event.data)
    return
  }

  switch (event.type) {
    case 'ResourceList': {
      if (event.data.type !== 'NamespacesUpdated') {
        const latest = d.read('lists')[listKeyOf(env.route)]?.latestRequest
        if (isSuperseded(env, latest)) return
      }
      handleList(d, env, event.data)
      return
    }
    case 'ResourceRead': {
      const latest = d.read('details')[readKeyOf(env, event.data)]?.latestRequest
      if (isSuperseded(env, latest)) return
      handleRead(d, env, event.data)
      return
    }
    case 'Stream':
      handleStream(d, event.data)
      return
  }
}

/** Rule 2 (FR-016): a `Request` correlation older than the latest one applied is superseded. */
function isSuperseded(env: BackendEventEnvelope, latestRequest: number | undefined): boolean {
  return (
    env.correlation.type === 'Request' &&
    latestRequest !== undefined &&
    env.correlation.data < latestRequest
  )
}

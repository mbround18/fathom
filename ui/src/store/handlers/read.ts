/** `BackendEvent::ResourceRead` arm: one-object reads (details, raw JSON for YAML, metrics). */
import type { BackendEventEnvelope, ResourceReadEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { detailKey } from '../keys'

/** `PodDetailsUpdated` → `Pod`; `PodMetricsHistoryUpdated` → `PodMetricsHistory`. */
export function readKind(type: ResourceReadEvent['type']): string {
  return type.replace(/(Details)?Updated$/, '')
}

export function readKeyOf(env: BackendEventEnvelope, event: ResourceReadEvent): string {
  const data = event.data
  const ns = 'namespace' in data ? data.namespace : null
  return detailKey(env.route.context, readKind(event.type), ns, data.name)
}

export function handleRead(d: Draft, env: BackendEventEnvelope, event: ResourceReadEvent): void {
  const key = readKeyOf(env, event)
  const prev = d.read('details')[key]
  const latest = env.correlation.type === 'Request' ? env.correlation.data : prev?.latestRequest
  d.edit('details')[key] = {
    kind: readKind(event.type),
    eventType: event.type,
    data: event.data,
    ...(latest !== undefined ? { latestRequest: latest } : {}),
    updatedAt: Date.now(),
  }
}

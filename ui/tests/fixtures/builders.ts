/**
 * Small constructors for wire-shaped fixtures. Everything here returns plain JSON data typed by
 * the generated protocol, so a fixture that drifts from the Rust types fails `tsc`.
 */
import type {
  BackendEvent,
  BackendEventEnvelope,
  Correlation,
  EventBatch,
  ResourceListEvent,
  ResourceReadEvent,
  Route,
  StreamEvent,
  SystemEvent,
} from '../../src/protocol/types'

export const SECTIONS = {
  system: 'system',
  events: 'events',
  pods: 'workloads/pods',
  deployments: 'workloads/deployments',
} as const

export const NO_CORRELATION: Correlation = { type: 'None' }

/** `Route::global(section)`: System events, cluster events. */
export function globalRoute(section: string = SECTIONS.system): Route {
  return { section, scope: 'Global', context: null, namespace: null }
}

/** `Route::cluster(section, context)`: e.g. NamespacesUpdated. */
export function clusterRoute(context: string, section: string = SECTIONS.system): Route {
  return { section, scope: 'Cluster', context, namespace: null }
}

/** `Route::namespaced(section, context, ns)`: lists use `ns = null`, reads the object's ns. */
export function nsRoute(context: string, section: string, namespace: string | null = null): Route {
  return { section, scope: 'Namespaced', context, namespace }
}

export function envelope(
  seq: number,
  route: Route,
  event: BackendEvent,
  correlation: Correlation = NO_CORRELATION,
): BackendEventEnvelope {
  return { version: 1, seq, route, correlation, event }
}

export function system(seq: number, data: SystemEvent): BackendEventEnvelope {
  return envelope(seq, globalRoute(), { type: 'System', data })
}

export function list(
  seq: number,
  route: Route,
  data: ResourceListEvent,
  correlation?: Correlation,
): BackendEventEnvelope {
  return envelope(seq, route, { type: 'ResourceList', data }, correlation)
}

export function read(
  seq: number,
  route: Route,
  data: ResourceReadEvent,
  correlation?: Correlation,
): BackendEventEnvelope {
  return envelope(seq, route, { type: 'ResourceRead', data }, correlation)
}

export function stream(seq: number, route: Route, data: StreamEvent): BackendEventEnvelope {
  return envelope(seq, route, { type: 'Stream', data })
}

export function batch(envelopes: BackendEventEnvelope[], dropped = 0): EventBatch {
  return { envelopes, dropped }
}

/**
 * 036-incluster-daemon: `DaemonStatusUpdated` / `DaemonEventsBatch` / `DaemonTopologyUpdated` /
 * `DaemonMetricsUpdated` handling. Per context; a batch tagged with a session older than the
 * newest seen for that context is a superseded stream and is discarded.
 */
import type {
  DaemonStatus,
  DaemonTimelineEntry,
  MetricSeries,
  TopologyDelta,
  TopologyEdge,
  TopologyNode,
  TruncationMarker,
} from '@/protocol/types'

import type { Draft } from '../draft'

/** Client-side timeline bound (matches the backend buffer). */
export const TIMELINE_CAP = 2000

export interface DaemonTopology {
  seq: number
  nodes: Record<string, TopologyNode>
  edges: TopologyEdge[]
}

export interface DaemonContextState {
  status: DaemonStatus | null
  /** Newest stream session seen; older-tagged events are dropped. */
  session: number
  /** Ascending by seq, unique by seq. */
  timeline: DaemonTimelineEntry[]
  truncated: TruncationMarker | null
  topology: DaemonTopology
  /** Keyed `${uid}|${metric}`. */
  metrics: Record<string, MetricSeries>
}

export type DaemonSlice = Record<string, DaemonContextState>

export const initialDaemon = (): DaemonSlice => ({})

const emptyTopology = (): DaemonTopology => ({ seq: 0, nodes: {}, edges: [] })

const emptyContext = (): DaemonContextState => ({
  status: null,
  session: 0,
  timeline: [],
  truncated: null,
  topology: emptyTopology(),
  metrics: {},
})

export const metricKey = (uid: string, metric: string): string => `${uid}|${metric}`

/** Merge `incoming` into `timeline`: dedupe by seq (later wins), keep ascending, cap. */
export function mergeTimeline(
  timeline: readonly DaemonTimelineEntry[],
  incoming: readonly DaemonTimelineEntry[],
): DaemonTimelineEntry[] {
  if (incoming.length === 0) return timeline as DaemonTimelineEntry[]
  const bySeq = new Map<number, DaemonTimelineEntry>()
  for (const e of timeline) bySeq.set(e.seq, e)
  for (const e of incoming) bySeq.set(e.seq, e)
  const merged = [...bySeq.values()].sort((a, b) => a.seq - b.seq)
  return merged.length > TIMELINE_CAP ? merged.slice(merged.length - TIMELINE_CAP) : merged
}

const edgeKey = (e: TopologyEdge): string => `${e.from}>${e.to}>${e.relation}`

export function applyTopologyDelta(t: DaemonTopology, delta: TopologyDelta): DaemonTopology {
  if (delta.seq < t.seq) return t
  const gone = new Set(delta.removed)
  const nodes = Object.fromEntries(Object.entries(t.nodes).filter(([uid]) => !gone.has(uid)))
  for (const n of delta.added) nodes[n.uid] = n
  const edges = new Map(t.edges.map((e) => [edgeKey(e), e]))
  for (const e of delta.edges.removed) edges.delete(edgeKey(e))
  for (const e of delta.edges.added) edges.set(edgeKey(e), e)
  // Edges whose endpoints vanished are dropped (no placeholders).
  const live = [...edges.values()].filter((e) => nodes[e.from] && nodes[e.to])
  return { seq: delta.seq, nodes, edges: live }
}

function ctx(d: Draft, context: string): DaemonContextState {
  const slice = d.edit('daemon')
  const cur = slice[context] ?? emptyContext()
  return cur
}

function put(d: Draft, context: string, next: DaemonContextState): void {
  const slice = d.edit('daemon')
  slice[context] = next
}

export function applyDaemonStatus(d: Draft, data: { context: string; status: DaemonStatus }): void {
  const cur = ctx(d, data.context)
  const usable = data.status.state === 'available'
  put(d, data.context, {
    ...cur,
    status: data.status,
    // Topology/metrics are only trusted while the daemon is up; otherwise the UI falls back.
    topology: usable ? cur.topology : emptyTopology(),
    metrics: usable ? cur.metrics : {},
    timeline: data.status.state === 'off' ? [] : cur.timeline,
    truncated: data.status.state === 'off' ? null : cur.truncated,
  })
}

export function applyDaemonBatch(
  d: Draft,
  data: {
    context: string
    session: number
    events: DaemonTimelineEntry[]
    truncated: TruncationMarker | null
  },
): void {
  const cur = ctx(d, data.context)
  if (data.session < cur.session) return
  let timeline = cur.timeline
  if (data.truncated?.reason === 'epochChanged') timeline = []
  put(d, data.context, {
    ...cur,
    session: data.session,
    timeline: mergeTimeline(timeline, data.events),
    truncated: data.truncated ?? cur.truncated,
  })
}

export function applyDaemonTopology(
  d: Draft,
  data: { context: string; session: number; delta: TopologyDelta },
): void {
  const cur = ctx(d, data.context)
  if (data.session < cur.session) return
  // A newer session starts from a full snapshot.
  const base = data.session > cur.session ? emptyTopology() : cur.topology
  put(d, data.context, {
    ...cur,
    session: data.session,
    topology: applyTopologyDelta(base, data.delta),
  })
}

export function applyDaemonMetrics(
  d: Draft,
  data: { context: string; session: number; series: MetricSeries[] },
): void {
  const cur = ctx(d, data.context)
  if (data.session < cur.session) return
  const metrics = { ...cur.metrics }
  for (const s of data.series) metrics[metricKey(s.key.objectUid, s.key.metric)] = s
  put(d, data.context, { ...cur, session: data.session, metrics })
}

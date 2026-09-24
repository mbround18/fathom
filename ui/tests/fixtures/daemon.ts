/** 036-incluster-daemon fixtures: statuses, timeline batches, topology and metric series. */
import type {
  DaemonCapabilities,
  DaemonState,
  DaemonTimelineEntry,
  EventBatch,
  MetricSeries,
  TopologyDelta,
  TruncationMarker,
} from '../../src/protocol/types'
import { batch, system } from './builders'

export const DAEMON_CONTEXT = 'kind-a'

export const daemonCaps = (over: Partial<DaemonCapabilities> = {}): DaemonCapabilities => ({
  protocolMajor: 1,
  protocolMinor: 0,
  daemonVersion: '0.1.0',
  epoch: 'e1',
  kinds: ['Deployment', 'Pod'],
  scope: { mode: 'cluster', namespaces: [] },
  retention: { maxEvents: 10000, maxAgeSecs: 86400 },
  snapshots: [],
  synced: true,
  topology: true,
  metrics: 'available',
  perUserFiltering: true,
  ...over,
})

export function daemonStatus(
  state: DaemonState,
  detail: string | null = null,
  context = DAEMON_CONTEXT,
): EventBatch {
  return batch([
    system(100, {
      type: 'DaemonStatusUpdated',
      data: {
        context,
        status: {
          context,
          state,
          detail,
          capabilities: state === 'available' ? daemonCaps() : null,
          lastEventAt: null,
          nextRetryAt: null,
        },
      },
    }),
  ])
}

export function entry(
  seq: number,
  over: Partial<DaemonTimelineEntry['event']> = {},
): DaemonTimelineEntry {
  return {
    seq,
    time: `2026-09-24T10:00:${String(seq % 60).padStart(2, '0')}Z`,
    event: {
      origin: 'derived',
      kind: 'Deployment',
      namespace: 'shop',
      name: 'web',
      uid: `u-${String(seq)}`,
      category: 'rollout',
      severity: 'error',
      reason: 'ProgressDeadlineExceeded',
      message: 'ReplicaSet "web-7d9" has timed out progressing.',
      repeat: 1,
      ...over,
    },
  }
}

export function daemonBatch(
  session: number,
  events: DaemonTimelineEntry[],
  truncated: TruncationMarker | null = null,
  context = DAEMON_CONTEXT,
): EventBatch {
  return batch([
    system(200 + session * 10 + events.length, {
      type: 'DaemonEventsBatch',
      data: { context, session, events, truncated },
    }),
  ])
}

export function topologyBatch(
  session: number,
  delta: TopologyDelta,
  context = DAEMON_CONTEXT,
): EventBatch {
  return batch([
    system(300 + delta.seq, {
      type: 'DaemonTopologyUpdated',
      data: { context, session, delta },
    }),
  ])
}

export const sampleTopology = (seq = 1): TopologyDelta => ({
  seq,
  added: [
    { uid: 'dep1', kind: 'Deployment', namespace: 'shop', name: 'web', health: 'ok' },
    { uid: 'pod1', kind: 'Pod', namespace: 'shop', name: 'web-abc', health: 'warning' },
    { uid: 'node1', kind: 'Node', namespace: null, name: 'kind-control-plane', health: 'ok' },
  ],
  removed: [],
  edges: {
    added: [
      { from: 'dep1', to: 'pod1', relation: 'owns' },
      { from: 'pod1', to: 'node1', relation: 'scheduledOn' },
    ],
    removed: [],
  },
})

export function metricsBatch(
  session: number,
  series: MetricSeries[],
  context = DAEMON_CONTEXT,
): EventBatch {
  return batch([
    system(400 + session, {
      type: 'DaemonMetricsUpdated',
      data: { context, session, series },
    }),
  ])
}

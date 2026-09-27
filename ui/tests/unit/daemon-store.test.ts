/** 036 T021: daemon slice — timeline merge/dedupe, staleness discard, topology, metrics. */
import { beforeEach, describe, expect, it } from 'vitest'

import { TOOL_LABEL } from '@/components/shell/labels'
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { mergeTimeline, metricKey } from '@/store/handlers/daemon'
import { ingest } from '@/store/ingest'

import {
  DAEMON_CONTEXT,
  daemonBatch,
  daemonStatus,
  entry,
  metricsBatch,
  sampleTopology,
  topologyBatch,
} from '../fixtures/daemon'

const d = () => useStore.getState().daemon[DAEMON_CONTEXT]

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
})

describe('mergeTimeline', () => {
  it('dedupes by seq, keeps ascending order, later wins', () => {
    const a = [entry(1), entry(3)]
    const b = [entry(2), entry(3, { reason: 'Changed' })]
    const m = mergeTimeline(a, b)
    expect(m.map((e) => e.seq)).toEqual([1, 2, 3])
    expect(m[2]?.event.reason).toBe('Changed')
  })

  it('caps the buffer to the newest 2000', () => {
    const many = Array.from({ length: 2100 }, (_, i) => entry(i + 1))
    const m = mergeTimeline([], many)
    expect(m).toHaveLength(2000)
    expect(m[0]?.seq).toBe(101)
  })
})

describe('daemon events', () => {
  it('records status and defaults to off when absent', () => {
    expect(d()).toBeUndefined()
    ingest(daemonStatus('notInstalled'))
    expect(d()?.status?.state).toBe('notInstalled')
  })

  it('a replayed batch after resume does not duplicate entries', () => {
    ingest(daemonBatch(1, [entry(1), entry(2)]))
    ingest(daemonBatch(2, [entry(2), entry(3)]))
    expect(d()?.timeline.map((e) => e.seq)).toEqual([1, 2, 3])
    expect(d()?.session).toBe(2)
  })

  it('discards batches from a superseded session', () => {
    ingest(daemonBatch(2, [entry(5)]))
    ingest(daemonBatch(1, [entry(6)]))
    expect(d()?.timeline.map((e) => e.seq)).toEqual([5])
  })

  it('truncation sets the banner marker; epoch change clears the old timeline', () => {
    ingest(daemonBatch(1, [entry(1), entry(2)]))
    ingest(
      daemonBatch(1, [entry(10)], {
        oldestSeq: 10,
        oldestTime: '2026-09-24T10:00:10Z',
        dropped: 0,
        reason: 'epochChanged',
      }),
    )
    expect(d()?.truncated?.reason).toBe('epochChanged')
    expect(d()?.timeline.map((e) => e.seq)).toEqual([10])
  })

  it('cursorTooOld keeps earlier entries but flags the gap', () => {
    ingest(daemonBatch(1, [entry(1)]))
    ingest(
      daemonBatch(1, [entry(50)], {
        oldestSeq: 40,
        oldestTime: '2026-09-24T10:00:40Z',
        dropped: 39,
        reason: 'cursorTooOld',
      }),
    )
    expect(d()?.timeline.map((e) => e.seq)).toEqual([1, 50])
    expect(d()?.truncated?.dropped).toBe(39)
  })

  it('turning the daemon off clears the timeline', () => {
    ingest(daemonBatch(1, [entry(1)]))
    ingest(daemonStatus('off'))
    expect(d()?.timeline).toEqual([])
  })
})

describe('daemon topology and metrics', () => {
  it('applies snapshot then deltas, dropping edges of removed nodes', () => {
    ingest(daemonStatus('available'))
    ingest(topologyBatch(1, sampleTopology(1)))
    expect(Object.keys(d()?.topology.nodes ?? {})).toHaveLength(3)
    expect(d()?.topology.edges).toHaveLength(2)
    ingest(
      topologyBatch(1, {
        seq: 2,
        added: [],
        removed: ['pod1'],
        edges: { added: [], removed: [] },
      }),
    )
    expect(Object.keys(d()?.topology.nodes ?? {})).toEqual(['dep1', 'node1'])
    expect(d()?.topology.edges).toHaveLength(0)
  })

  it('ignores stale deltas and resets on a newer session', () => {
    ingest(daemonStatus('available'))
    ingest(topologyBatch(1, sampleTopology(5)))
    ingest(topologyBatch(1, { ...sampleTopology(3), added: [] }))
    expect(d()?.topology.seq).toBe(5)
    ingest(
      topologyBatch(2, {
        seq: 1,
        added: sampleTopology().added.slice(0, 1),
        removed: [],
        edges: { added: [], removed: [] },
      }),
    )
    expect(Object.keys(d()?.topology.nodes ?? {})).toEqual(['dep1'])
  })

  it('drops topology when the daemon becomes unavailable (fallback)', () => {
    ingest(daemonStatus('available'))
    ingest(topologyBatch(1, sampleTopology()))
    ingest(daemonStatus('unavailable', 'connection refused'))
    expect(Object.keys(d()?.topology.nodes ?? {})).toHaveLength(0)
  })

  it('stores metric series with the truncation marker', () => {
    ingest(daemonStatus('available'))
    ingest(
      metricsBatch(1, [
        {
          key: { objectUid: 'pod1', metric: 'cpu' },
          samples: [{ time: '2026-09-24T10:00:00Z', value: 1 }],
          truncatedOldestTime: '2026-09-24T09:30:00Z',
        },
      ]),
    )
    expect(d()?.metrics[metricKey('pod1', 'cpu')]?.truncatedOldestTime).toBe('2026-09-24T09:30:00Z')
  })
})

describe('setDaemonPreference', () => {
  it('sends the command and reflects the toggle optimistically', () => {
    const sent: unknown[] = []
    useStore.setState({ send: (c) => void sent.push(c) })
    useStore.getState().setDaemonPreference(DAEMON_CONTEXT, 'auto')
    expect(sent).toEqual([
      { type: 'SetDaemonPreference', data: { context: DAEMON_CONTEXT, mode: 'auto' } },
    ])
    expect(d()?.status?.state).toBe('probing')
    useStore.getState().setDaemonPreference(DAEMON_CONTEXT, 'off')
    expect(d()?.status?.state).toBe('off')
  })
})

describe('043 US4 naming', () => {
  it('names the daemon dock tool "Cluster history"', () => {
    expect(TOOL_LABEL.daemon).toBe('Cluster history')
  })
})

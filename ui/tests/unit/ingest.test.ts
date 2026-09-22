/**
 * Reducer tests (T034, data-model.md §3 rules 1–5). Each case starts from a fresh store.
 */
import { toast } from 'sonner'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { DiskUsage, EventBatch, PodSummary } from '@/protocol/types'
// `@/store` must be imported before any descriptor module that (transitively, via
// `resources/related.tsx`) imports `@/store` itself — otherwise the reentrant cycle leaves
// that descriptor's module mid-evaluation and `ALL_DESCRIPTORS` picks up `undefined` for it.
// The app's own entry point (`main.tsx`) naturally imports `@/store` first; this just matches
// that order.
import { eventsFor, initialData, listKey, type LogStreamState, useStore } from '@/store'
import { pods as podDescriptor } from '@/resources/pods'
import { resetActions, runAction } from '@/store/actions'
import { ingest } from '@/store/ingest'

import {
  clusterEvents,
  contextChanged,
  DETAIL_POD,
  droppedWarning,
  noKubeconfig,
  PODS_500,
  pods,
  podDetailKindA,
  podsError,
  PODS_FORBIDDEN,
  requestNewer,
  requestOlder,
  staleKindA,
  STALE_POD_NAME,
  twoContexts,
  uiConfigDefault,
  uiConfigSaved,
  SAVED_UI,
} from '../fixtures'
import { batch, clusterRoute, envelope, nsRoute, stream, system } from '../fixtures/builders'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

const DISK: DiskUsage = {
  namespace: 'default',
  name: 'data-0',
  provisioned_bytes: 1024,
  used_bytes: 512,
  state: 'Bound',
}

const PODS_A = listKey('kind-a', 'workloads/pods', 'Namespaced', null)
const PODS_B = listKey('kind-b', 'workloads/pods', 'Namespaced', null)

function podRows(key = PODS_A): readonly PodSummary[] {
  return (useStore.getState().lists[key]?.rows ?? []) as readonly PodSummary[]
}

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  vi.clearAllMocks()
})

describe('session', () => {
  it('ContextsUpdated/ContextChanged set the context list and the active context', () => {
    ingest(twoContexts)
    const { session } = useStore.getState()
    expect(session.contexts).toEqual(['kind-a', 'kind-b'])
    expect(session.contextsLoaded).toBe(true)
    expect(session.activeContext).toBe('kind-a')
  })

  it('no kubeconfig: empty contexts, no active context, error shown verbatim', () => {
    ingest(noKubeconfig)
    const { session, conn } = useStore.getState()
    expect(session.contexts).toEqual([])
    expect(session.contextsLoaded).toBe(true)
    expect(session.activeContext).toBeUndefined()
    expect(conn.lastNotice?.level).toBe('error')
    expect(conn.lastNotice?.message).toMatch(/^failed to infer config/)
  })

  it('UiConfigLoaded with ui=null keeps defaults; a saved config replaces them', () => {
    const defaults = useStore.getState().ui
    ingest(uiConfigDefault)
    expect(useStore.getState().ui).toBe(defaults)
    expect(useStore.getState().prefs).toEqual({
      loaded: true,
      expandedGroups: ['Workloads'],
      mcpAutoStart: false,
    })
    ingest(uiConfigSaved)
    expect(useStore.getState().ui).toEqual(SAVED_UI)
    expect(useStore.getState().prefs.mcpAutoStart).toBe(true)
  })

  it('an error notification is surfaced verbatim', () => {
    ingest(podsError)
    expect(useStore.getState().conn.lastNotice).toMatchObject({
      level: 'error',
      message: PODS_FORBIDDEN,
    })
  })
})

describe('rule 1: context staleness', () => {
  it('drops non-System envelopes for a context that is not active', () => {
    ingest(twoContexts)
    ingest(pods('kind-b'))
    expect(useStore.getState().lists[PODS_B]).toBeUndefined()
  })

  it('drops a late kind-a list after switching to kind-b', () => {
    ingest(twoContexts)
    useStore.getState().selectContext('kind-b')
    ingest(staleKindA)
    ingest(contextChanged('kind-b'))
    ingest(pods('kind-b'))
    expect(useStore.getState().session.activeContext).toBe('kind-b')
    expect(podRows(PODS_A).some((p) => p.name === STALE_POD_NAME)).toBe(false)
    expect(podRows(PODS_B).length).toBeGreaterThan(0)
  })

  it('keeps System envelopes regardless of context', () => {
    ingest(twoContexts)
    ingest(batch([system(9, { type: 'Notification', data: { level: 'Warn', message: 'hi' } })]))
    expect(useStore.getState().conn.lastNotice?.level).toBe('warn')
  })

  it('keys cluster events by their originating context', () => {
    ingest(twoContexts)
    ingest(clusterEvents())
    const events = useStore.getState().lists[listKey('kind-a', 'events', 'Cluster', null)]
    expect(events?.rows).toHaveLength(3)
    expect(events?.total).toBe(3)
  })

  it('drops a System event bound to a context that is no longer active', () => {
    ingest(twoContexts)
    ingest(contextChanged('kind-b'))
    const stale: EventBatch = batch([
      envelope(70, clusterRoute('kind-a'), {
        type: 'System',
        data: { type: 'DiskUsageUpdated', data: [DISK] },
      }),
    ])
    ingest(stale)
    expect(useStore.getState().metrics.diskUsage).toEqual([])

    ingest(
      batch([
        envelope(71, clusterRoute('kind-b'), {
          type: 'System',
          data: { type: 'DiskUsageUpdated', data: [DISK] },
        }),
      ]),
    )
    expect(useStore.getState().metrics.diskUsage).toEqual([DISK])
  })

  it('never renders cluster events from the previous context after a switch (FR-016)', () => {
    ingest(twoContexts)
    ingest(contextChanged('kind-b'))
    ingest(clusterEvents(undefined, 80, 'kind-a'))
    const s = useStore.getState()
    expect(s.lists[listKey('kind-a', 'events', 'Cluster', null)]).toBeUndefined()
    expect(eventsFor(s, 'Pod', DETAIL_POD.namespace, DETAIL_POD.name)).toEqual([])
  })
})

describe('rule 2: superseded requests (FR-016)', () => {
  it('drops a list reply whose request id is older than the latest applied', () => {
    ingest(twoContexts)
    ingest(requestNewer)
    ingest(requestOlder)
    const list = useStore.getState().lists[PODS_A]
    expect(list?.latestRequest).toBe(8)
    expect(list?.rows).toHaveLength(2)
  })

  it('applies an equal or newer request id', () => {
    ingest(twoContexts)
    ingest(requestOlder)
    ingest(requestNewer)
    expect(useStore.getState().lists[PODS_A]?.rows).toHaveLength(2)
  })
})

describe('rule 3: row identity', () => {
  it('reuses unchanged row objects across refreshes', () => {
    ingest(twoContexts)
    ingest(pods('kind-a'))
    const before = podRows()
    const changed = PODS_500.map((p, i) =>
      i === 3 ? { ...p, restarts: p.restarts + 1 } : { ...p },
    )
    ingest(pods('kind-a', changed))
    const after = podRows()
    expect(after).toHaveLength(before.length)
    expect(after[0]).toBe(before[0])
    expect(after[3]).not.toBe(before[3])
    expect(after[3]?.restarts).toBe((before[3]?.restarts ?? 0) + 1)
  })

  it('keeps the rows array itself when nothing changed', () => {
    ingest(twoContexts)
    ingest(pods('kind-a'))
    const before = podRows()
    ingest(
      pods(
        'kind-a',
        PODS_500.map((p) => ({ ...p })),
      ),
    )
    expect(podRows()).toBe(before)
  })
})

describe('rule 4: streams', () => {
  it('appends log chunks into a bounded ring buffer', () => {
    ingest(twoContexts)
    const route = nsRoute('kind-a', 'workloads/pods', 'default')
    const lines = Array.from({ length: 6000 }, (_, i) => `l${i}`)
    ingest(batch([stream(1, route, { type: 'PodLogsChunk', data: { stream_id: 4, lines } })]))
    const first = useStore.getState().streams[4]
    ingest(batch([stream(2, route, { type: 'PodLogsChunk', data: { stream_id: 4, lines } })]))
    const s = useStore.getState().streams[4] as LogStreamState | undefined
    expect(s).not.toBe(first)
    expect(s?.lines.length).toBe(10_000)
    expect(s?.lines.truncated).toBe(2_000)
    ingest(
      batch([stream(3, route, { type: 'StreamClosed', data: { stream_id: 4, reason: 'EOF' } })]),
    )
    expect(useStore.getState().streams[4]).toMatchObject({ status: 'closed', reason: 'EOF' })
  })

  it('switching the selected container restarts the stream under a new stream_id, independent of the old one (T010)', () => {
    ingest(twoContexts)
    const route = nsRoute('kind-a', 'workloads/pods', 'default')
    // Logs tab opens against the first container: backend assigns stream_id 7.
    ingest(batch([stream(10, route, { type: 'PodLogsChunk', data: { stream_id: 7, lines: ['a'] } })]))
    // Operator switches container: PodLogsTab sends StopPodLogs + StartPodLogs, a fresh
    // stream_id (8) is assigned for the new container's stream.
    ingest(batch([stream(11, route, { type: 'PodLogsChunk', data: { stream_id: 8, lines: ['b'] } })]))
    // A late StreamClosed for the superseded stream_id 7 must not touch stream_id 8's state.
    ingest(
      batch([stream(12, route, { type: 'StreamClosed', data: { stream_id: 7, reason: 'EOF' } })]),
    )
    const streams = useStore.getState().streams
    expect(streams[7]).toMatchObject({ status: 'closed', reason: 'EOF' })
    const next = streams[8] as LogStreamState | undefined
    expect(next?.status).toBe('open')
    expect(next?.lines.toArray()).toEqual(['b'])
  })
})

function commandFinishedEnv(seq: number, requestId: number, ok: boolean, error: string | null) {
  return envelope(
    seq,
    clusterRoute('kind-b'),
    { type: 'CommandFinished', data: { ok, error } },
    {
      type: 'Request',
      data: requestId,
    },
  )
}

describe('rule 5: pending actions', () => {
  it('CommandFinished resolves the action awaiting that request id, whatever its context', async () => {
    useStore.setState({ request: () => Promise.resolve(12) })
    const [action] = podDescriptor.actions
    const rows = PODS_500.slice(0, 2)
    if (!action) throw new Error('pods have a delete action')
    const run = runAction(podDescriptor, action, rows, { paneId: 'list-1' })
    await vi.waitFor(() => expect(useStore.getState().pending[12]).toBeDefined())
    ingest(batch([commandFinishedEnv(5, 12, true, null)]))
    const outcome = await run
    expect(outcome.ok).toHaveLength(2)
    expect(useStore.getState().pending).toEqual({})
  })

  it("a pending action's error notification is only a notice: pending, selection untouched", () => {
    useStore.setState({
      pending: { 3: { label: 'Delete pod', paneId: 'list-1', sentAt: 0 } },
      selection: { 'list-1': ['default/a'] },
    })
    ingest(
      batch([
        {
          ...system(5, { type: 'Notification', data: { level: 'Error', message: 'forbidden' } }),
          correlation: { type: 'Request', data: 3 },
        },
      ]),
    )
    const s = useStore.getState()
    expect(s.selection).toEqual({ 'list-1': ['default/a'] })
    expect(s.pending[3]).toBeDefined()
    expect(s.conn.lastNotice?.message).toBe('forbidden')
    expect(toast.error).not.toHaveBeenCalled()
  })

  it('an error correlated with a refresh (not an action) still fails the loading lists', () => {
    ingest(twoContexts)
    useStore.getState().refreshSection('workloads/pods')
    ingest(
      batch([
        {
          ...system(6, { type: 'Notification', data: { level: 'Error', message: PODS_FORBIDDEN } }),
          correlation: { type: 'Request', data: 41 },
        },
      ]),
    )
    expect(useStore.getState().lists[PODS_A]).toMatchObject({
      status: 'error',
      error: PODS_FORBIDDEN,
    })
  })
})

describe('reads', () => {
  it('stores pod details under the detail key, including raw_json', () => {
    ingest(twoContexts)
    ingest(podDetailKindA)
    const key = `kind-a|Pod|${DETAIL_POD.namespace}|${DETAIL_POD.name}`
    const detail = useStore.getState().details[key]
    expect(detail?.kind).toBe('Pod')
    expect(detail?.data.name).toBe(DETAIL_POD.name)
  })
})

describe('batch semantics', () => {
  it('one batch produces exactly one store notification', () => {
    ingest(twoContexts)
    const listener = vi.fn()
    const unsubscribe = useStore.subscribe(listener)
    const big: EventBatch = batch([
      ...contextChanged('kind-a').envelopes,
      ...pods('kind-a').envelopes,
      ...clusterEvents().envelopes,
    ])
    ingest(big)
    unsubscribe()
    expect(listener).toHaveBeenCalledTimes(1)
  })

  it('version !== 1 sets the fatal state and ignores the batch', () => {
    ingest(twoContexts)
    const bad = pods('kind-a')
    const env = bad.envelopes[0]
    if (!env) throw new Error('fixture has no envelope')
    ingest({ ...bad, envelopes: [{ ...env, version: 2 }] })
    const s = useStore.getState()
    expect(s.conn.status).toBe('backend-gone')
    expect(s.conn.fatal).toMatch(/protocol version 2/)
    expect(s.lists[PODS_A]).toBeUndefined()
  })

  it('dropped > 0 accumulates into conn.dropped', () => {
    ingest(droppedWarning)
    ingest(droppedWarning)
    expect(useStore.getState().conn.dropped).toBe(6)
  })

  it('untouched slices keep their identity', () => {
    const before = useStore.getState()
    ingest(twoContexts)
    const after = useStore.getState()
    expect(after.session).not.toBe(before.session)
    expect(after.lists).toBe(before.lists)
    expect(after.panes).toBe(before.panes)
  })
})

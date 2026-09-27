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
import { CLIENT_STREAM_ID_MIN } from '@/store/log-streams'

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

  it('no kubeconfig: empty contexts, no active context, the Missing issue and no raw error', () => {
    ingest(noKubeconfig)
    const { session, conn } = useStore.getState()
    expect(session.contexts).toEqual([])
    expect(session.contextsLoaded).toBe(true)
    expect(session.activeContext).toBeUndefined()
    expect(session.kubeconfigIssue?.kind).toBe('Missing')
    expect(conn.lastNotice).toBeUndefined()
  })

  it('UiConfigLoaded with ui=null keeps defaults; a saved config replaces them', () => {
    const defaults = useStore.getState().ui
    ingest(uiConfigDefault)
    expect(useStore.getState().ui).toBe(defaults)
    expect(useStore.getState().prefs).toEqual({
      loaded: true,
      expandedGroups: ['Workloads'],
      mcpStdioEnabled: false,
    })
    ingest(uiConfigSaved)
    expect(useStore.getState().ui).toEqual(SAVED_UI)
    expect(useStore.getState().prefs.mcpStdioEnabled).toBe(true)
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
  const route = nsRoute('kind-a', 'workloads/pods', 'default')
  const target = { namespace: 'default', name: 'web', container: null }
  const logs = (id: number) => useStore.getState().streams[id] as LogStreamState | undefined
  const sent: unknown[] = []

  beforeEach(() => {
    sent.length = 0
    ingest(twoContexts)
    useStore.setState({
      send: (cmd) => void sent.push(cmd),
      request: (cmd) => {
        sent.push(cmd)
        return Promise.resolve(1)
      },
    })
  })

  it('startPodLogs seeds a starting entry and sends StartPodLogs with its client id', () => {
    const id = useStore.getState().startPodLogs(target, 'view-1')
    expect(id).toBeGreaterThanOrEqual(CLIENT_STREAM_ID_MIN)
    expect(Number.isSafeInteger(id)).toBe(true)
    expect(logs(id)).toMatchObject({ kind: 'logs', status: 'starting', owner: 'view-1', target })
    expect(sent).toEqual([{ type: 'StartPodLogs', data: { ...target, stream_id: id } }])
  })

  it('PodLogsOpened with no output is streaming and empty, not starting', () => {
    const id = useStore.getState().startPodLogs(target)
    ingest(batch([stream(1, route, { type: 'PodLogsOpened', data: { stream_id: id } })]))
    expect(logs(id)?.status).toBe('streaming')
    expect(logs(id)?.lines.length).toBe(0)
  })

  it('appends log chunks into a bounded ring buffer', () => {
    const id = useStore.getState().startPodLogs(target)
    const lines = Array.from({ length: 6000 }, (_, i) => `l${i}`)
    ingest(batch([stream(1, route, { type: 'PodLogsChunk', data: { stream_id: id, lines } })]))
    const first = logs(id)
    // A chunk proves the stream is live even without its PodLogsOpened.
    expect(first?.status).toBe('streaming')
    ingest(batch([stream(2, route, { type: 'PodLogsChunk', data: { stream_id: id, lines } })]))
    const s = logs(id)
    expect(s).not.toBe(first)
    expect(s?.lines.length).toBe(10_000)
    expect(s?.lines.truncated).toBe(2_000)
    ingest(
      batch([stream(3, route, { type: 'StreamClosed', data: { stream_id: id, reason: 'ended' } })]),
    )
    expect(logs(id)).toMatchObject({ status: 'closed', reason: 'ended' })
  })

  it('a StreamClosed with an error is the error state, with the verbatim message', () => {
    const id = useStore.getState().startPodLogs(target)
    ingest(
      batch([
        stream(1, route, {
          type: 'StreamClosed',
          data: { stream_id: id, reason: 'error', error: 'pods "web" not found' },
        }),
      ]),
    )
    expect(logs(id)).toMatchObject({ status: 'error', error: 'pods "web" not found' })
  })

  it('a refused StartPodLogs (backend gone, hosted limit) is the error state', async () => {
    useStore.setState({ request: () => Promise.reject(new Error('stream limit reached')) })
    const id = useStore.getState().startPodLogs(target)
    await vi.waitFor(() => expect(logs(id)?.status).toBe('error'))
    expect(logs(id)?.error).toBe('stream limit reached')
  })

  it('concurrent streams are independent; stopping one forgets only it', () => {
    const { startPodLogs, stopPodLogs } = useStore.getState()
    const a = startPodLogs(target, 'inspector')
    const b = startPodLogs(target, 'dock')
    expect(a).not.toBe(b)
    ingest(
      batch([
        stream(1, route, { type: 'PodLogsChunk', data: { stream_id: a, lines: ['a'] } }),
        stream(2, route, { type: 'PodLogsChunk', data: { stream_id: b, lines: ['b'] } }),
      ]),
    )
    stopPodLogs(a)
    expect(sent.at(-1)).toEqual({ type: 'StopPodLogStream', data: { stream_id: a } })
    expect(logs(a)).toBeUndefined()
    // The stopped stream's late events neither resurrect it nor touch the other stream.
    ingest(
      batch([
        stream(3, route, { type: 'PodLogsChunk', data: { stream_id: a, lines: ['late'] } }),
        stream(4, route, { type: 'StreamClosed', data: { stream_id: a, reason: 'stopped' } }),
      ]),
    )
    expect(logs(a)).toBeUndefined()
    expect(logs(b)).toMatchObject({ status: 'streaming' })
    expect(logs(b)?.lines.toArray()).toEqual(['b'])
  })

  it('events for a stream this client never started are ignored', () => {
    const foreign = CLIENT_STREAM_ID_MIN + 99
    ingest(
      batch([
        stream(1, route, { type: 'PodLogsOpened', data: { stream_id: foreign } }),
        stream(2, route, { type: 'PodLogsChunk', data: { stream_id: foreign, lines: ['x'] } }),
        stream(3, route, { type: 'StreamClosed', data: { stream_id: foreign, reason: 'ended' } }),
      ]),
    )
    expect(useStore.getState().streams).toEqual({})
  })

  it('a re-open of a live stream (hosted recycle) starts a fresh buffer', () => {
    const id = useStore.getState().startPodLogs(target)
    ingest(
      batch([
        stream(1, route, { type: 'PodLogsOpened', data: { stream_id: id } }),
        stream(2, route, { type: 'PodLogsChunk', data: { stream_id: id, lines: ['a'] } }),
        stream(3, route, { type: 'PodLogsOpened', data: { stream_id: id } }),
        stream(4, route, { type: 'PodLogsChunk', data: { stream_id: id, lines: ['b'] } }),
      ]),
    )
    expect(logs(id)?.lines.toArray()).toEqual(['b'])
  })

  it('an exec StreamClosed for an unseen backend id still seeds the exec error entry', () => {
    ingest(
      batch([stream(1, route, { type: 'StreamClosed', data: { stream_id: 3, reason: 'denied' } })]),
    )
    expect(useStore.getState().streams[3]).toMatchObject({ kind: 'exec', status: 'error' })
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

describe('section-routed refresh errors', () => {
  it("fails only the routed section's loading lists", () => {
    ingest(twoContexts)
    useStore.getState().refreshSection('workloads/pods')
    useStore.getState().refreshSection('workloads/deployments')
    const DEPLOYS_A = listKey('kind-a', 'workloads/deployments', 'Namespaced', null)
    const base = system(7, {
      type: 'Notification',
      data: { level: 'Error', message: PODS_FORBIDDEN },
    })
    ingest(
      batch([
        {
          ...base,
          route: { ...base.route, section: 'workloads/pods' },
          correlation: { type: 'Request', data: 42 },
        },
      ]),
    )
    const lists = useStore.getState().lists
    expect(lists[PODS_A]).toMatchObject({ status: 'error', error: PODS_FORBIDDEN })
    expect(lists[DEPLOYS_A]?.status).toBe('loading')
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

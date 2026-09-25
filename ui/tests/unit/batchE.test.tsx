import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import EventsPanel from '@/components/panels/EventsPanel'
import OverviewPanel, { summarize } from '@/components/panels/OverviewPanel'
import { filterEvents, newestFirst, relativeTime } from '@/components/panels/eventsModel'
import type { KubernetesEvent, NodeSummary, PodSummary } from '@/protocol/types'
import { descriptorFor } from '@/resources'
import { listKey, useStore } from '@/store'
import { handleList } from '@/store/handlers/list'

const ev = (over: Partial<KubernetesEvent>): KubernetesEvent => ({
  namespace: 'default',
  name: 'e1',
  event_type: 'Normal',
  involved_object_kind: 'Pod',
  involved_object_name: 'web-1',
  reason: 'Started',
  message: 'Started container',
  count: 1,
  first_timestamp: '2026-01-01T00:00:00Z',
  last_timestamp: '2026-01-01T00:00:00Z',
  source: 'kubelet',
  ...over,
})

const EVENTS = [
  ev({ name: 'a', last_timestamp: '2026-01-01T00:00:00Z' }),
  ev({
    name: 'b',
    event_type: 'Warning',
    reason: 'BackOff',
    message: 'Back-off restarting',
    count: 7,
    namespace: 'payments',
    involved_object_name: 'pay-1',
    last_timestamp: '2026-01-01T00:10:00Z',
  }),
  ev({
    name: 'c',
    event_type: 'Warning',
    reason: 'Unhealthy',
    last_timestamp: '2026-01-01T00:05:00Z',
  }),
]

function seed(events: KubernetesEvent[], extra: Record<string, unknown> = {}) {
  const openInspector = vi.fn()
  useStore.setState({
    session: {
      ...useStore.getState().session,
      activeContext: 'ctx',
      contexts: ['ctx'],
      contextsLoaded: true,
    },
    conn: { status: 'ready', dropped: 0 },
    lists: {
      [listKey('ctx', 'events', 'Cluster', null)]: {
        eventType: 'EventsUpdated',
        rows: events,
        total: events.length,
        status: 'ready',
        updatedAt: 0,
      },
    },
    refreshEvents: () => undefined,
    openInspector,
    ...extra,
  } as never)
  return openInspector
}

describe('events model', () => {
  it('sorts newest first and filters', () => {
    expect(newestFirst(EVENTS).map((e) => e.name)).toEqual(['b', 'c', 'a'])
    expect(filterEvents(EVENTS, { type: 'warning', namespace: null, search: '' })).toHaveLength(2)
    expect(filterEvents(EVENTS, { type: 'all', namespace: 'payments', search: '' })).toHaveLength(1)
    expect(filterEvents(EVENTS, { type: 'all', namespace: null, search: 'back-off' })).toHaveLength(
      1,
    )
  })
  it('formats relative times', () => {
    const now = Date.parse('2026-01-01T01:00:00Z')
    expect(relativeTime('2026-01-01T00:59:30Z', now)).toBe('30s')
    expect(relativeTime('2026-01-01T00:50:00Z', now)).toBe('10m')
    expect(relativeTime('2026-01-01T00:00:00Z', now)).toBe('1h')
    expect(relativeTime('nope', now)).toBe('—')
  })
})

describe('EventsPanel', () => {
  beforeEach(() => {
    useStore.setState({ layout: { ...useStore.getState().layout, widthClass: 'laptop' } } as never)
  })

  it('lists newest first, filters by type, namespace and search', () => {
    seed(EVENTS)
    render(<EventsPanel />)
    const names = () =>
      [...document.querySelectorAll('[data-event-row]')].map((n) =>
        n.getAttribute('data-event-row'),
      )
    expect(names()).toEqual(['payments/b', 'default/c', 'default/a'])
    fireEvent.click(screen.getByRole('button', { name: 'Warning' }))
    expect(names()).toEqual(['payments/b', 'default/c'])
    fireEvent.change(screen.getByLabelText('Namespace'), { target: { value: 'payments' } })
    expect(names()).toEqual(['payments/b'])
    fireEvent.change(screen.getByLabelText('Namespace'), { target: { value: '' } })
    fireEvent.change(screen.getByLabelText('Search events'), { target: { value: 'unhealthy' } })
    expect(names()).toEqual(['default/c'])
  })

  it('shows the count and opens the involved object in the inspector', () => {
    const open = seed(EVENTS)
    render(<EventsPanel />)
    expect(screen.getByText('×7')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Open Pod/pay-1' }))
    expect(open).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'Pod', name: 'pay-1', namespace: 'payments' }),
    )
  })

  it('renders cards on phone', () => {
    seed(EVENTS)
    useStore.setState({ layout: { ...useStore.getState().layout, widthClass: 'phone' } } as never)
    render(<EventsPanel />)
    expect(document.querySelectorAll('li[data-event-row]')).toHaveLength(3)
  })

  it('shows an empty state when there are no events', () => {
    seed([])
    render(<EventsPanel />)
    expect(screen.getByText('No events')).toBeTruthy()
  })
})

describe('OverviewPanel', () => {
  const node = (name: string, status: string): NodeSummary => ({
    name,
    status,
    role: 'worker',
    pod_count: 0,
    pod_capacity: 10,
    cpu_allocatable_millicores: 1,
    memory_allocatable_bytes: 1,
  })
  const pod = (name: string, phase: string): PodSummary => ({
    name,
    namespace: 'default',
    phase,
    node: 'n',
    ready: 1,
    total: 1,
    restarts: 0,
  })

  it('summarizes unhealthy counts', () => {
    const c = summarize(
      [node('a', 'Ready'), node('b', 'NotReady')],
      [pod('p1', 'Running'), pod('p2', 'Pending'), pod('p3', 'CrashLoopBackOff')],
      [
        { name: 'd', namespace: 'x', desired: 3, ready: 2, updated: 3, available: 2 },
        { name: 'e', namespace: 'x', desired: 1, ready: 1, updated: 1, available: 1 },
      ],
    )
    expect(c).toMatchObject({
      nodes: 2,
      nodesUnhealthy: 1,
      pods: 3,
      podsUnhealthy: 2,
      deployments: 2,
      deploymentsUnhealthy: 1,
    })
  })

  it('renders counts, version, namespaces and recent warnings', () => {
    seed(EVENTS, {
      clusterVersion: {
        server_version: 'v1.30.1',
        node_kubelet_versions: [],
        skew_detected: false,
      },
    })
    useStore.setState({
      session: { ...useStore.getState().session, namespaces: ['a', 'b', 'c'] },
      lists: {
        ...useStore.getState().lists,
        [listKey('ctx', 'nodes', 'Cluster', null)]: {
          rows: [node('a', 'Ready')],
          status: 'ready',
          updatedAt: 0,
        },
      },
    } as never)
    render(<OverviewPanel />)
    const panel = screen.getByTestId('overview-panel')
    expect(within(panel).getByText('v1.30.1')).toBeTruthy()
    expect(panel.querySelector('[data-stat="namespaces"]')?.textContent).toContain('3')
    expect(panel.querySelector('[data-stat="nodes"]')?.textContent).toContain('1')
    expect(within(panel).getByText('BackOff')).toBeTruthy()
    expect(within(panel).queryByText('Started')).toBeNull()
  })

  it('is empty only when disconnected', () => {
    seed([])
    useStore.setState({
      session: { ...useStore.getState().session, activeContext: undefined },
    } as never)
    render(<OverviewPanel />)
    expect(screen.getByText('Not connected')).toBeTruthy()
  })
})

describe('namespaces', () => {
  it('has a descriptor with a SelectNamespace read and ingests summaries under the namespaces key', () => {
    const d = descriptorFor('namespaces')
    expect(d?.kind).toBe('Namespace')
    expect(
      d?.read?.({ section: 'namespaces', kind: 'Namespace', namespace: null, name: 'x' })[0],
    ).toEqual({
      type: 'SelectNamespace',
      data: { name: 'x' },
    })
    const writes: Record<string, unknown> = {}
    const d2 = {
      read: () => ({}),
      edit: () => writes,
    } as never
    handleList(
      d2,
      {
        route: { context: 'ctx', section: 'system', scope: 'Cluster', namespace: null },
        correlation: { type: 'None' },
      } as never,
      {
        type: 'NamespaceSummariesUpdated',
        data: [
          {
            name: 'default',
            status: 'Active',
            pod_count: 2,
            deployment_count: 1,
            creation_timestamp: '',
          },
        ],
      },
    )
    expect(Object.keys(writes)).toEqual([listKey('ctx', 'namespaces', 'Cluster', null)])
  })
})

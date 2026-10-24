import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { Cmd, Container, EventEnvelope, HostSample, Stack, StackList, UsageSample } from '@/protocol/generated'

// The store talks to the shared socket only through these three calls.
const bus = vi.hoisted(() => ({
  sent: [] as Cmd[],
  socket: true,
  listeners: [] as ((e: EventEnvelope) => void)[],
  reconnect: [] as (() => void)[],
}))
vi.mock('@/app/connection', () => ({
  sendCmd: (cmd: Cmd) => {
    if (!bus.socket) return null
    bus.sent.push(cmd)
    return bus.sent.length
  },
  onServerEvent: (l: (e: EventEnvelope) => void) => {
    bus.listeners.push(l)
    return () => undefined
  },
  onReconnected: (l: () => void) => {
    bus.reconnect.push(l)
    return () => undefined
  },
  connect: () => undefined,
  disconnect: () => undefined,
}))

const { useSession } = await import('@/app/session-store')
const { resetHostStore, useHost, watchHost } = await import('@/features/stacks/host-store')
const m = await import('@/features/stacks/metrics')

const GiB = 1024 ** 3

function host(over: Partial<HostSample> = {}): HostSample {
  return {
    at: '2026-10-23T12:00:00Z',
    cpu_percent: 25,
    cpu_cores: 4,
    load: { one: 2, five: 1, fifteen: 0.5 },
    memory: { total_bytes: 8 * GiB, available_bytes: 6 * GiB, used_bytes: 2 * GiB },
    swap: { total_bytes: 0, free_bytes: 0, used_bytes: 0 },
    uptime_secs: 3600,
    disk: { total_bytes: 100 * GiB, used_bytes: 40 * GiB, free_bytes: 60 * GiB },
    containers_running: 1,
    containers_stopped: 0,
    ...over,
  }
}

function container(id: string, state = 'running'): Container {
  return {
    id,
    name: id,
    image: 'x',
    state,
    health: null,
    status: '',
    created_at: '2026-10-23T12:00:00Z',
    command: '',
    ports: [],
    project: null,
    service: null,
    cluster: null,
    adoption: 'adopted',
  } as Container
}

function stack(name: string, containers: Container[]): Stack {
  return {
    name,
    valid_name: true,
    compose_file: 'compose.yaml',
    shadowed_files: [],
    has_env: false,
    content_hash: `h-${name}`,
    status: 'running',
    unhealthy: false,
    services: [{ name: 'app', containers }],
    running_services: 1,
    total_services: 1,
    modified_at: '2026-10-23T12:00:00Z',
    read_error: null,
    invalid_reason: null,
    read_only_reason: null,
    too_large: false,
  } as unknown as Stack
}

function list(stacks: Stack[]): StackList {
  return { root: '/srv/stacks', stacks, foreign: [], standalone: [], warnings: [], stale: false, as_of: '2026-10-23T12:00:00Z' }
}

function sample(container_id: string, cpu: number | null, mem: number | null): UsageSample {
  return { container_id, at: '2026-10-23T12:00:00Z', cpu_percent: cpu, memory_bytes: mem, memory_limit_bytes: null }
}

function emit(env: EventEnvelope): void {
  for (const l of bus.listeners) l(env)
}

describe('formatting', () => {
  it('formats bytes with binary units', () => {
    expect(m.formatBytes(0)).toBe('0 B')
    expect(m.formatBytes(1536)).toBe('1.5 KiB')
    expect(m.formatBytes(20 * 1024 * 1024)).toBe('20 MiB')
    expect(m.formatBytes(8 * GiB)).toBe('8.0 GiB')
    expect(m.formatBytes(null)).toBe('—')
    expect(m.formatBytes(-1)).toBe('—')
  })

  it('formats percentages and uptime', () => {
    expect(m.formatPercent(12.345)).toBe('12.3%')
    expect(m.formatPercent(null)).toBe('—')
    expect(m.formatUptime(42)).toBe('42s')
    expect(m.formatUptime(300)).toBe('5m')
    expect(m.formatUptime(3 * 3600 + 120)).toBe('3h 2m')
    expect(m.formatUptime(90_061)).toBe('1d 1h')
    expect(m.formatUptime(null)).toBe('—')
  })

  it('turns a sample into 0–100 figures, null where unknown', () => {
    expect(m.hostPoint(host())).toEqual({ cpu: 25, memory: 25, disk: 40, load: 50 })
    expect(m.hostPoint(host({ cpu_percent: null, memory: null, disk: null, cpu_cores: null }))).toEqual({
      cpu: null,
      memory: null,
      disk: null,
      load: null,
    })
    expect(m.hostPoint(host({ load: { one: 9, five: 0, fifteen: 0 } })).load).toBe(100)
  })

  it('draws a sparkline right-aligned in the window, breaking at gaps', () => {
    expect(m.sparkPath([], 100, 10)).toBe('')
    expect(m.sparkPath([0, 100], 100, 10, 3)).toBe('M50.0 10.0L100.0 0.0')
    expect(m.sparkPath([50, null, 50], 100, 10, 3)).toBe('M0.0 5.0M100.0 5.0')
    expect(m.sparkPath([150], 100, 10, 1)).toBe('M0.0 0.0')
  })
})

describe('per-stack usage', () => {
  const l = list([
    stack('web', [container('web-1'), container('web-2')]),
    stack('db', [container('db-1')]),
    stack('idle', [container('idle-1', 'exited')]),
    stack('empty', []),
  ])

  it('sums each stack’s containers and sorts busiest first', () => {
    const rows = m.stackUsages(l, {
      'web-1': sample('web-1', 10, 100),
      'web-2': sample('web-2', 5, 50),
      'db-1': sample('db-1', 40, 1000),
    })
    expect(rows.map((r) => [r.name, r.cpu, r.memory, r.measured])).toEqual([
      ['db', 40, 1000, 1],
      ['web', 15, 150, 2],
      ['idle', null, null, 0],
    ])
  })

  it('matches samples by id prefix, either way', () => {
    const rows = m.stackUsages(l, { web: sample('web', 3, 1) })
    expect(rows.find((r) => r.name === 'web')?.cpu).toBe(6)
  })

  it('watches only stacks with a running container, capped', () => {
    expect(m.watchedStacks(l, 64)).toEqual(['db', 'web'])
    expect(m.watchedStacks(l, 1)).toEqual(['db'])
    expect(m.watchedStacks(null, 64)).toEqual([])
  })
})

describe('host store', () => {
  beforeEach(() => {
    bus.sent.length = 0
    bus.socket = true
    resetHostStore()
  })

  it('watches once for every holder and unwatches after the last', () => {
    const a = watchHost()
    const b = watchHost()
    expect(bus.sent).toEqual([{ type: 'WatchHost' }])
    a()
    a()
    expect(bus.sent).toHaveLength(1)
    b()
    expect(bus.sent).toEqual([{ type: 'WatchHost' }, { type: 'UnwatchHost' }])
  })

  it('keeps the latest sample and a bounded history', () => {
    const stop = watchHost()
    for (let i = 0; i < m.HOST_HISTORY + 5; i++) emit({ event: { type: 'HostSample', data: host({ cpu_percent: i }) } })
    const s = useHost.getState()
    expect(s.latest?.cpu_percent).toBe(m.HOST_HISTORY + 4)
    expect(s.history).toHaveLength(m.HOST_HISTORY)
    expect(s.history[0]?.cpu).toBe(5)
    stop()
    emit({ event: { type: 'HostSample', data: host({ cpu_percent: 99 }) } })
    expect(useHost.getState().latest?.cpu_percent).toBe(m.HOST_HISTORY + 4)
  })

  it('records a denial and stops asking', () => {
    const stop = watchHost()
    emit({ reply_to: 1, event: { type: 'Denied', data: { reason: 'feature_disabled' } } })
    expect(useHost.getState().denied).toBe('feature_disabled')
    stop()
    const again = watchHost()
    expect(bus.sent).toEqual([{ type: 'WatchHost' }])
    again()
  })

  it('ignores a denial of some other request', () => {
    const stop = watchHost()
    emit({ reply_to: 42, event: { type: 'Denied', data: { reason: 'not_permitted' } } })
    expect(useHost.getState().denied).toBeNull()
    stop()
  })

  it('waits for a socket, and re-sends the watch after a reconnect', () => {
    bus.socket = false
    const stop = watchHost()
    expect(bus.sent).toEqual([])
    bus.socket = true
    useSession.setState({ connection: 'open' })
    expect(bus.sent).toEqual([{ type: 'WatchHost' }])
    for (const r of bus.reconnect) r()
    expect(bus.sent).toEqual([{ type: 'WatchHost' }, { type: 'WatchHost' }])
    stop()
    useSession.setState({ connection: 'closed' })
  })

  it('lets go while the page is hidden', () => {
    const stop = watchHost()
    const vis = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    expect(bus.sent.at(-1)).toEqual({ type: 'UnwatchHost' })
    vis.mockReturnValue('visible')
    document.dispatchEvent(new Event('visibilitychange'))
    expect(bus.sent.at(-1)).toEqual({ type: 'WatchHost' })
    vis.mockRestore()
    stop()
  })

  it('forgets everything when the session ends', () => {
    const stop = watchHost()
    emit({ event: { type: 'HostSample', data: host() } })
    useSession.setState({ status: 'ready' })
    useSession.setState({ status: 'signed_out' })
    expect(useHost.getState()).toMatchObject({ latest: null, history: [], denied: null })
    stop()
  })
})

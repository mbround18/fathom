import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { Cmd, EventEnvelope, Stack, StackList } from '@/protocol/generated'

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
const { resetStacksStore, useStacks } = await import('@/features/stacks/store')
const model = await import('@/features/stacks/model')

function stack(name: string, over: Partial<Stack> = {}): Stack {
  return {
    name,
    valid_name: true,
    compose_file: 'compose.yaml',
    shadowed_files: [],
    has_env: false,
    content_hash: `h-${name}`,
    status: 'running',
    unhealthy: false,
    services: [],
    running_services: 1,
    total_services: 1,
    modified_at: '2026-10-23T12:00:00Z',
    read_error: null,
    invalid_reason: null,
    read_only_reason: null,
    too_large: false,
    ...over,
  }
}

function list(stacks: Stack[], over: Partial<StackList> = {}): StackList {
  return { root: '/srv/stacks', stacks, foreign: [], standalone: [], warnings: [], stale: false, as_of: '2026-10-23T12:00:00Z', ...over }
}

function emit(env: EventEnvelope): void {
  for (const l of bus.listeners) l(env)
}

beforeEach(() => {
  bus.sent.length = 0
  bus.socket = true
  resetStacksStore()
})

describe('model', () => {
  const stacks = [stack('web'), stack('db', { status: 'stopped' }), stack('webhooks', { status: 'partial' })]

  it('filters by name, case-insensitively, and by status', () => {
    expect(model.filterStacks(stacks, 'WEB', 'all').map((s) => s.name)).toEqual(['web', 'webhooks'])
    expect(model.filterStacks(stacks, '', 'stopped').map((s) => s.name)).toEqual(['db'])
    expect(model.filterStacks(stacks, 'web', 'partial').map((s) => s.name)).toEqual(['webhooks'])
  })

  it('hides foreign and standalone entries under a status filter', () => {
    const items = [{ name: 'adhoc' }, { name: 'other' }]
    expect(model.filterByName(items, 'ad', 'all')).toEqual([{ name: 'adhoc' }])
    expect(model.filterByName(items, '', 'running')).toEqual([])
  })

  it('says why a stack is blocked', () => {
    expect(model.blockedReason(stack('ok'))).toBeNull()
    expect(model.blockedReason(stack('x', { read_error: 'Permission denied (os error 13)' }))).toBe('Permission denied (os error 13)')
    expect(model.blockedReason(stack('Bad Name', { valid_name: false, status: 'invalid' }))).toBe('Not a valid compose project name')
    expect(model.blockedReason(stack('broken', { status: 'invalid' }))).toBe('The compose file does not parse')
  })

  it('names both files when one shadows another (FR-004)', () => {
    const notes = model.stackNotes(stack('w', { shadowed_files: ['docker-compose.yml'] }), [])
    expect(notes.find((n) => n.key === 'shadowed')?.text).toContain('compose.yaml')
    expect(notes.find((n) => n.key === 'shadowed')?.text).toContain('docker-compose.yml')
  })

  it('masks secret-looking keys only', () => {
    const text = '# c\nPORT=8080\nDB_PASSWORD=hunter2\napi_token = abc\nexport AWS_SECRET_ACCESS_KEY=x\nEMPTY_TOKEN=\n'
    expect(model.maskEnv(text)).toBe(
      '# c\nPORT=8080\nDB_PASSWORD=********\napi_token = ********\nexport AWS_SECRET_ACCESS_KEY=********\nEMPTY_TOKEN=\n',
    )
  })

  it('formats ports and collapses duplicate bindings', () => {
    const p = { container_port: 80, host_port: 8080, host_ip: '0.0.0.0', protocol: 'tcp' }
    expect(model.formatPorts([p, { ...p, host_ip: '::' }, { container_port: 9000, host_port: null, host_ip: null, protocol: 'tcp' }])).toEqual([
      '8080→80/tcp',
      '9000/tcp',
    ])
  })
})

describe('stacks store', () => {
  it('sends ListStacks once and takes the reply and live updates', () => {
    useStacks.getState().load()
    useStacks.getState().load()
    expect(bus.sent).toEqual([{ type: 'ListStacks' }])
    emit({ reply_to: 1, event: { type: 'Stacks', data: list([stack('web')]) } })
    expect(useStacks.getState().list?.stacks.map((s) => s.name)).toEqual(['web'])
    emit({ event: { type: 'StacksChanged', data: list([stack('web'), stack('db')]) } })
    expect(useStacks.getState().list?.stacks.map((s) => s.name)).toEqual(['web', 'db'])
  })

  it('keeps the last snapshot, marked stale, when the runtime is unreachable', () => {
    useStacks.getState().load()
    emit({ reply_to: 1, event: { type: 'Stacks', data: list([stack('web')]) } })
    emit({ event: { type: 'RuntimeUnavailable', data: { reason: 'Cannot connect to the Docker daemon' } } })
    const s = useStacks.getState()
    expect(s.runtimeError).toBe('Cannot connect to the Docker daemon')
    expect(s.list?.stale).toBe(true)
    expect(s.list?.stacks).toHaveLength(1)
  })

  it('marks a denied GetStack as denied, and refreshes an open detail on a live update', () => {
    useStacks.getState().openDetail('web')
    useStacks.getState().openDetail('secret')
    expect(bus.sent).toEqual([
      { type: 'GetStack', data: { name: 'web' } },
      { type: 'GetStack', data: { name: 'secret' } },
    ])
    emit({ reply_to: 2, event: { type: 'Denied', data: { reason: 'not_permitted' } } })
    expect(useStacks.getState().details.secret).toEqual({ state: 'denied' })

    const web = stack('web')
    emit({ reply_to: 1, event: { type: 'StackDetail', data: { stack: web, compose_text: 'services: {}\n', env: null, content_hash: web.content_hash } } })
    emit({ event: { type: 'StacksChanged', data: list([stack('web', { status: 'stopped' })]) } })
    const d = useStacks.getState().details.web
    expect(d?.state === 'ready' && d.detail.stack.status).toBe('stopped')
    expect(bus.sent).toHaveLength(2)

    emit({ event: { type: 'StacksChanged', data: list([stack('web', { content_hash: 'h-new' })]) } })
    expect(bus.sent.at(-1)).toEqual({ type: 'GetStack', data: { name: 'web' } })
  })

  it('re-issues ListStacks on reconnect, and sends it once the socket opens if it had none', () => {
    bus.socket = false
    useStacks.getState().load()
    expect(bus.sent).toEqual([])
    bus.socket = true
    useSession.setState({ connection: 'open' })
    expect(bus.sent).toEqual([{ type: 'ListStacks' }])
    for (const l of bus.reconnect) l()
    expect(bus.sent).toEqual([{ type: 'ListStacks' }, { type: 'ListStacks' }])
  })

  it('forgets everything when the session leaves the signed-in state', () => {
    useSession.setState({ status: 'ready' })
    useStacks.getState().load()
    emit({ reply_to: 1, event: { type: 'Stacks', data: list([stack('web')]) } })
    useSession.setState({ status: 'signed_out' })
    expect(useStacks.getState().list).toBeNull()
  })
})

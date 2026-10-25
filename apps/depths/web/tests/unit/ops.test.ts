import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { Cmd, Event, EventEnvelope, Preview } from '@/protocol/generated'

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

const { useOps, resetOpsStore } = await import('@/features/stacks/ops-store')
const { argvLine, quoteArg, refusalText } = await import('@/features/stacks/argv')

function emit(event: Event, reply_to?: number): void {
  for (const l of bus.listeners) l(reply_to === undefined ? { event } : { reply_to, event })
}

function preview(over: Partial<Preview> = {}): Preview {
  return {
    preview_id: 'pv-1',
    stack: 'web',
    kind: 'up',
    steps: [{ argv: ['docker', 'compose', 'up', '-d'], cwd: '/srv/stacks/web', stdin: 'none' }],
    file_diff: null,
    reaches: { containers: ['web-app-1'], networks: ['web_default'], volumes: [] },
    destructive: false,
    stack_hash: 'h1',
    expires_at: '2026-10-23T12:05:00Z',
    ...over,
  }
}

const up = { kind: 'op', stack: 'web', op: { kind: 'up' } } as const

beforeEach(() => {
  bus.sent.length = 0
  bus.socket = true
  resetOpsStore()
})

describe('argv rendering', () => {
  it('leaves plain words alone and single-quotes the rest', () => {
    expect(quoteArg('compose.yaml')).toBe('compose.yaml')
    expect(quoteArg('')).toBe("''")
    expect(quoteArg('a b')).toBe("'a b'")
    expect(quoteArg("it's")).toBe(`'it'\\''s'`)
    expect(quoteArg('$HOME')).toBe("'$HOME'")
    expect(argvLine(['docker', 'compose', '-p', 'my stack'])).toBe("docker compose -p 'my stack'")
  })
})

describe('ops store', () => {
  it('sends PreviewOp and shows the reply', () => {
    useOps.getState().requestPreview(up)
    expect(bus.sent).toEqual([{ type: 'PreviewOp', data: { stack: 'web', op: { kind: 'up' } } }])
    expect(useOps.getState().flow?.phase).toBe('previewing')
    emit({ type: 'Preview', data: preview() }, 1)
    expect(useOps.getState().flow).toMatchObject({ phase: 'ready', preview: { preview_id: 'pv-1' } })
  })

  it('ignores a reply to a superseded preview request', () => {
    useOps.getState().requestPreview(up)
    useOps.getState().requestPreview({ kind: 'op', stack: 'web', op: { kind: 'stop' } })
    emit({ type: 'Preview', data: preview({ preview_id: 'old' }) }, 1)
    expect(useOps.getState().flow?.phase).toBe('previewing')
    emit({ type: 'Preview', data: preview({ preview_id: 'new', kind: 'stop' }) }, 2)
    expect(useOps.getState().flow?.preview?.preview_id).toBe('new')
  })

  it('confirms by preview id, then streams output and the outcome by operation id', () => {
    useOps.getState().requestPreview(up)
    emit({ type: 'Preview', data: preview() }, 1)
    useOps.getState().confirm()
    expect(bus.sent[1]).toEqual({ type: 'Confirm', data: { preview_id: 'pv-1' } })
    emit({ type: 'OpStarted', data: { operation_id: 'op-1' } }, 2)
    expect(useOps.getState().flow).toBeNull()
    emit({ type: 'OpOutput', data: { operation_id: 'op-1', chunk: 'one\n' } }, 2)
    emit({ type: 'OpOutput', data: { operation_id: 'op-1', chunk: 'two\n' } })
    emit({ type: 'OpOutput', data: { operation_id: 'other', chunk: 'not ours\n' } })
    expect(useOps.getState().ops.web?.text).toBe('one\ntwo\n')
    emit({ type: 'OpFinished', data: { operation_id: 'op-1', outcome: { kind: 'failed', exit_code: 1, stderr: 'boom' } } })
    expect(useOps.getState().ops.web?.outcome).toEqual({ kind: 'failed', exit_code: 1, stderr: 'boom' })
  })

  it('keeps the flow open with the refusal when the preview is void', () => {
    useOps.getState().requestPreview(up)
    emit({ type: 'Preview', data: preview() }, 1)
    useOps.getState().confirm()
    emit({ type: 'PreviewVoid', data: { reason: 'stack_changed' } }, 2)
    expect(useOps.getState().flow).toMatchObject({ phase: 'refused', refusal: { kind: 'void', reason: 'stack_changed' } })
    expect(useOps.getState().ops.web).toBeUndefined()
  })

  it('records who holds a busy stack', () => {
    useOps.getState().requestPreview(up)
    emit({ type: 'Busy', data: { stack: 'web', operation_id: 'op-9', kind: 'up', actor: { provider: 'discord', subject: '1', username: 'bo' } } }, 1)
    expect(useOps.getState().flow?.refusal).toEqual({
      kind: 'busy',
      stack: 'web',
      operationId: 'op-9',
      actor: { provider: 'discord', subject: '1', username: 'bo' },
      running: null,
    })
  })

  it('sends Cancel for the running operation', () => {
    useOps.getState().requestPreview(up)
    emit({ type: 'Preview', data: preview() }, 1)
    useOps.getState().confirm()
    emit({ type: 'OpStarted', data: { operation_id: 'op-1' } }, 2)
    useOps.getState().cancelOp('web')
    expect(bus.sent[2]).toEqual({ type: 'Cancel', data: { operation_id: 'op-1' } })
    expect(useOps.getState().ops.web?.cancelling).toBe(true)
    emit({ type: 'OpFinished', data: { operation_id: 'op-1', outcome: { kind: 'cancelled' } } }, 3)
    expect(useOps.getState().ops.web).toMatchObject({ cancelling: false, outcome: { kind: 'cancelled' } })
  })

  it('on reconnect, re-asks a pending preview and marks a lost confirm and running ops', () => {
    useOps.getState().requestPreview(up)
    emit({ type: 'Preview', data: preview() }, 1)
    useOps.getState().confirm()
    emit({ type: 'OpStarted', data: { operation_id: 'op-1' } }, 2)
    useOps.getState().requestPreview({ kind: 'op', stack: 'db', op: { kind: 'stop' } })
    for (const r of bus.reconnect) r()
    expect(bus.sent.at(-1)).toEqual({ type: 'PreviewOp', data: { stack: 'db', op: { kind: 'stop' } } })
    expect(useOps.getState().ops.web?.interrupted).toBe(true)

    emit({ type: 'Preview', data: preview({ stack: 'db', kind: 'stop', preview_id: 'pv-2' }) }, bus.sent.length)
    useOps.getState().confirm()
    for (const r of bus.reconnect) r()
    expect(useOps.getState().flow?.refusal).toEqual({ kind: 'lost' })
  })

  it('says the request was lost when there is no socket', () => {
    bus.socket = false
    useOps.getState().requestPreview(up)
    expect(useOps.getState().flow).toMatchObject({ phase: 'refused', refusal: { kind: 'lost' } })
  })
})

describe('051 compose findings', () => {
  it('sends acknowledge_findings only when asked, and carries a compose block detail', () => {
    useOps.getState().requestPreview(up)
    emit({ type: 'Preview', data: preview({ ack_required: true, warnings: [] }) }, 1)
    useOps.getState().confirm(true)
    expect(bus.sent[1]).toEqual({ type: 'Confirm', data: { preview_id: 'pv-1', acknowledge_findings: true } })
    emit({ type: 'Denied', data: { reason: 'compose_blocked', detail: 'app: privileged' } }, 2)
    const refusal = useOps.getState().flow?.refusal
    expect(refusal).toEqual({ kind: 'denied', reason: 'compose_blocked', detail: 'app: privileged' })
    if (!refusal) throw new Error('refused')
    expect(refusalText(refusal)).toMatch(/acknowledging the findings\. app: privileged$/)
  })
})

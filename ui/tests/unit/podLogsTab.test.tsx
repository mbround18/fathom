/**
 * PodLogsTab (feedback #2): each mounted view owns its own log stream, and every stream state
 * (starting, streaming-but-empty, lines, error, closed) and a failed details read render
 * distinctly instead of a skeleton forever.
 */
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// `@/store` must be imported before any descriptor module (see ingest.test.ts's note).
import { initialData, useStore } from '@/store'
import { resetActions } from '@/store/actions'
import { ingest } from '@/store/ingest'
import { PodLogsTab } from '@/components/panels/PodLogsTab'
import type { BackendCmd, StreamEvent } from '@/protocol/types'

import { DETAIL_POD, podDetailKindA, twoContexts } from '../fixtures'
import { batch, clusterRoute, envelope, nsRoute, stream } from '../fixtures/builders'

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), loading: vi.fn() } }))

const target = {
  section: 'workloads/pods',
  kind: 'Pod',
  namespace: DETAIL_POD.namespace,
  name: DETAIL_POD.name,
}
const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
let sent: BackendCmd[] = []
let seq = 1000

function emit(event: StreamEvent) {
  act(() => {
    ingest(batch([stream(seq++, route, event)]))
  })
}

function starts(): number[] {
  return sent.flatMap((c) =>
    c.type === 'StartPodLogs' && c.data.stream_id != null ? [c.data.stream_id] : [],
  )
}

function stops(): number[] {
  return sent.flatMap((c) => (c.type === 'StopPodLogStream' ? [c.data.stream_id] : []))
}

beforeEach(() => {
  resetActions()
  useStore.setState(initialData())
  ingest(twoContexts)
  sent = []
  useStore.setState({
    send: (cmd) => void sent.push(cmd),
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(sent.length)
    },
  })
})

describe('PodLogsTab', () => {
  it('starts its own stream, shows "No log output yet" once open, then lines', () => {
    ingest(podDetailKindA)
    render(<PodLogsTab target={target} />)
    expect(screen.getByRole('status', { name: 'Starting log stream' })).toBeInTheDocument()
    const [id] = starts()
    if (id === undefined) throw new Error('StartPodLogs carries a stream id')

    emit({ type: 'PodLogsOpened', data: { stream_id: id } })
    expect(screen.getByText('No log output yet.')).toBeInTheDocument()
    expect(screen.queryByRole('status', { name: 'Starting log stream' })).toBeNull()

    emit({ type: 'PodLogsChunk', data: { stream_id: id, lines: ['hello', 'world'] } })
    expect(screen.getByTestId('pod-logs-scroll')).toHaveTextContent('hello world')
    expect(screen.getByTestId('pod-logs-scroll')).toHaveAttribute('data-stream-status', 'streaming')
  })

  it('two mounted views stream independently; unmounting one stops only its stream', () => {
    ingest(podDetailKindA)
    const inspector = render(<PodLogsTab target={target} />)
    render(<PodLogsTab target={target} />)
    const [a, b] = starts()
    if (a === undefined || b === undefined) throw new Error('two streams')
    expect(a).not.toBe(b)
    emit({ type: 'PodLogsChunk', data: { stream_id: b, lines: ['from b'] } })

    inspector.unmount()
    expect(stops()).toEqual([a])
    // The surviving view still has its stream and lines.
    expect(screen.getByTestId('pod-logs-scroll')).toHaveTextContent('from b')
    emit({ type: 'PodLogsChunk', data: { stream_id: b, lines: ['more'] } })
    expect(screen.getByTestId('pod-logs-scroll')).toHaveTextContent('from b more')
  })

  it('a stream error shows the message inline, and Retry restarts under a new id', async () => {
    const user = userEvent.setup()
    ingest(podDetailKindA)
    render(<PodLogsTab target={target} />)
    const [id] = starts()
    if (id === undefined) throw new Error('stream id')
    emit({
      type: 'StreamClosed',
      data: { stream_id: id, reason: 'error', error: 'Failed to start logs: forbidden' },
    })
    expect(screen.getByRole('alert')).toHaveTextContent('Failed to start logs: forbidden')

    await user.click(screen.getByRole('button', { name: /retry/i }))
    expect(stops()).toEqual([id])
    const next = starts()[1]
    expect(next).toBeDefined()
    expect(next).not.toBe(id)
    expect(screen.getByRole('status', { name: 'Starting log stream' })).toBeInTheDocument()
  })

  it('an ended stream keeps its lines and says so', () => {
    ingest(podDetailKindA)
    render(<PodLogsTab target={target} />)
    const [id] = starts()
    if (id === undefined) throw new Error('stream id')
    emit({ type: 'PodLogsChunk', data: { stream_id: id, lines: ['bye'] } })
    emit({ type: 'StreamClosed', data: { stream_id: id, reason: 'ended' } })
    expect(screen.getByTestId('pod-logs-scroll')).toHaveTextContent('bye')
    expect(document.querySelector('[data-pane-state="stream-ended"]')).toHaveTextContent(
      'Stream ended',
    )
  })

  it('a failed pod-details read is an inline error with Retry, not a skeleton', async () => {
    const user = userEvent.setup()
    render(<PodLogsTab target={target} />)
    expect(starts()).toEqual([])
    const read = sent.findIndex((c) => c.type === 'SelectPod')
    expect(read).toBeGreaterThanOrEqual(0)
    const requestId = read + 1
    await act(async () => {
      await Promise.resolve()
      ingest(
        batch([
          envelope(
            seq++,
            clusterRoute('kind-a'),
            { type: 'CommandFinished', data: { ok: false, error: 'pods "x" is forbidden' } },
            { type: 'Request', data: requestId },
          ),
        ]),
      )
    })
    expect(await screen.findByText('pods "x" is forbidden')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /retry/i }))
    expect(sent.filter((c) => c.type === 'SelectPod')).toHaveLength(2)
  })
})

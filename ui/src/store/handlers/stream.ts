/** `BackendEvent::Stream` arm: log chunks into ring buffers (rule 4) and tunnel status. */
import type { StreamEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import type { LogStreamState } from '../index'
import { RingBuffer } from '../ring-buffer'

export function handleStream(d: Draft, event: StreamEvent): void {
  switch (event.type) {
    case 'PodLogsChunk': {
      const streams = d.edit('streams')
      const prev = streams[event.data.stream_id]
      const lines = prev?.lines ?? new RingBuffer<string>()
      lines.pushAll(event.data.lines)
      const next: LogStreamState = prev
        ? { ...prev, rev: prev.rev + 1 }
        : { kind: 'logs', lines, rev: 1, follow: true, status: 'open' }
      streams[event.data.stream_id] = next
      return
    }
    case 'StreamClosed': {
      const prev = d.read('streams')[event.data.stream_id]
      if (!prev) return
      d.edit('streams')[event.data.stream_id] = {
        ...prev,
        status: 'closed',
        reason: event.data.reason,
      }
      return
    }
    case 'PortForwardTunnel':
      d.edit('tunnels')[event.data.tunnel_id] = {
        kind: 'portforward',
        status: event.data.status,
        localEndpoint: event.data.local_endpoint,
      }
      return
  }
}

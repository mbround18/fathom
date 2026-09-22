/** `BackendEvent::Stream` arm: log chunks into ring buffers (rule 4) and tunnel status. */
import type { StreamEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { markApplied } from '../editSession'
import type { ExecStreamState, LogStreamState } from '../index'
import { RingBuffer } from '../ring-buffer'

/** Decode a base64-encoded backend byte chunk into a UTF-8/binary-safe string for xterm.js. */
function decodeExecChunk(base64: string): string {
  const binary = atob(base64)
  // xterm.js expects a "binary string" (Latin1-per-char), not UTF-8 decoding, so it can
  // preserve raw terminal escape/control bytes verbatim (research.md R4).
  return binary
}

export function handleStream(d: Draft, event: StreamEvent): void {
  switch (event.type) {
    case 'PodLogsChunk': {
      const streams = d.edit('streams')
      const prev = streams[event.data.stream_id]
      const logPrev = prev?.kind === 'logs' ? prev : undefined
      const lines = logPrev?.lines ?? new RingBuffer<string>()
      lines.pushAll(event.data.lines)
      const next: LogStreamState = logPrev
        ? { ...logPrev, rev: logPrev.rev + 1 }
        : { kind: 'logs', lines, rev: 1, follow: true, status: 'open' }
      streams[event.data.stream_id] = next
      return
    }
    case 'PodExecReady': {
      // Attach succeeded: create/flip the `kind: 'exec'` entry to 'active' (data-model.md
      // "Terminal Session" — 'attaching' itself is frontend-only, tracked by PodTerminalTab
      // before this event, since the backend assigns `stream_id` only on success).
      const streams = d.edit('streams')
      const prev = streams[event.data.stream_id]
      const execPrev = prev?.kind === 'exec' ? prev : undefined
      const next: ExecStreamState = {
        kind: 'exec',
        status: 'active',
        container: execPrev?.container,
        chunks: execPrev?.chunks ?? new RingBuffer<string>(),
        rev: execPrev?.rev ?? 0,
      }
      streams[event.data.stream_id] = next
      return
    }
    case 'PodExecOutputChunk': {
      const streams = d.edit('streams')
      const prev = streams[event.data.stream_id]
      const execPrev = prev?.kind === 'exec' ? prev : undefined
      const chunks = execPrev?.chunks ?? new RingBuffer<string>()
      chunks.push(decodeExecChunk(event.data.data))
      const next: ExecStreamState = execPrev
        ? { ...execPrev, chunks, rev: execPrev.rev + 1 }
        : { kind: 'exec', status: 'active', chunks, rev: 1 }
      streams[event.data.stream_id] = next
      return
    }
    case 'StreamClosed': {
      const prev = d.read('streams')[event.data.stream_id]
      if (!prev) {
        // An exec session that failed to attach (e.g. permission denied, no usable shell)
        // never gets a `PodExecReady`, so this is the first time its `stream_id` is seen —
        // seed an errored `kind: 'exec'` entry so PodTerminalTab can surface FR-013's
        // verbatim reason. A logs `stream_id` is always seeded earlier by `PodLogsChunk`,
        // so a missing entry there stays a no-op, as before.
        d.edit('streams')[event.data.stream_id] = {
          kind: 'exec',
          status: 'error',
          reason: event.data.reason,
          chunks: new RingBuffer<string>(),
          rev: 0,
        }
        return
      }
      d.edit('streams')[event.data.stream_id] =
        prev.kind === 'logs'
          ? { ...prev, status: 'closed', reason: event.data.reason }
          : {
              ...prev,
              status: event.data.reason === 'exit' ? 'ended' : 'error',
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
    case 'ResourceApplied': {
      // T022: the session awaiting this apply's request id (matched, not just "the" open
      // session, since the operator could in principle re-open the tab before this arrives).
      const sessions = d.read('editSessions')
      const key = Object.keys(sessions).find((k) => {
        const s = sessions[k]
        return s?.status === 'applying' && String(s.requestId) === event.data.request_id
      })
      if (!key) return
      const session = sessions[key]
      if (!session) return
      d.edit('editSessions')[key] = markApplied(session, event.data.resource_version)
      return
    }
    case 'DrainProgress': {
      const nodeActions = d.read('nodeActions')
      const nodeName = Object.keys(nodeActions).find(
        (n) => nodeActions[n]?.requestId === event.data.request_id,
      )
      if (!nodeName) return
      const prev = nodeActions[nodeName]
      if (!prev || prev.kind !== 'drain') return
      d.edit('nodeActions')[nodeName] = {
        ...prev,
        status: 'in-progress',
        drain: {
          totalPods: event.data.total_pods,
          evicted: event.data.evicted,
          blockedOn: event.data.blocked_on.map(([pod, reason]) => ({ pod, reason })),
          ignoredDaemonSetPods: event.data.ignored_daemonset_pods,
          cancelled: prev.drain?.cancelled ?? false,
        },
      }
      return
    }
    case 'NodeActionFinished': {
      const nodeActions = d.read('nodeActions')
      const nodeName = Object.keys(nodeActions).find(
        (n) => nodeActions[n]?.requestId === event.data.request_id,
      )
      if (!nodeName) return
      const prev = nodeActions[nodeName]
      if (!prev) return
      // A cancel already set `status: 'cancelled'` optimistically; a `NodeActionFinished` that
      // arrives afterwards for the same request must not downgrade it back to failed/complete.
      if (prev.status === 'cancelled') return
      d.edit('nodeActions')[nodeName] = {
        ...prev,
        status: event.data.ok ? 'complete' : 'failed',
        error: event.data.error ?? undefined,
      }
      return
    }
  }
}

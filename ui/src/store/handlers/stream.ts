/** `BackendEvent::Stream` arm: log stream states and chunks into ring buffers (rule 4), exec
 * sessions, and tunnel status. */
import { resetCapabilities } from '@/lib/capabilities'
import type { StreamEvent } from '@/protocol/types'

import type { Draft } from '../draft'
import { handleContainers, isContainerEvent } from './containers'
import { markApplied } from '../editSession'
import { type ExecStreamState, type LogStreamState, useStore } from '../index'
import { isClientStreamId } from '../log-streams'
import { RingBuffer } from '@companion/terminal'

/** Decode a base64-encoded backend byte chunk into a UTF-8/binary-safe string for xterm.js. */
function decodeExecChunk(base64: string): string {
  const binary = atob(base64)
  // xterm.js expects a "binary string" (Latin1-per-char), not UTF-8 decoding, so it can
  // preserve raw terminal escape/control bytes verbatim (research.md R4).
  return binary
}

export function handleStream(d: Draft, event: StreamEvent): void {
  // 048-docker-management: the local container runtime's events reduce in their own handler, for
  // the same reason the backend gates them in one place — this family is about the computer, not
  // the cluster, and mixing it into the switch below is how an arm gets missed.
  if (isContainerEvent(event)) {
    handleContainers(d, event)
    return
  }
  switch (event.type) {
    // Log streams exist from `startPodLogs` on (feedback #2). Events for an id this client did
    // not start, or already stopped (another tab of a shared hosted backend, a late chunk after
    // `StopPodLogStream`), are ignored rather than seeding an entry nobody will ever clean up.
    case 'PodLogsOpened': {
      const prev = d.read('streams')[event.data.stream_id]
      if (prev?.kind !== 'logs') return
      // A (re)open replays the tail, so it starts a fresh buffer (a hosted recycle restarts a
      // live id in place).
      const next: LogStreamState = {
        ...prev,
        lines: prev.status === 'starting' ? prev.lines : new RingBuffer<string>(),
        rev: prev.rev + 1,
        status: 'streaming',
        reason: undefined,
        error: undefined,
      }
      d.edit('streams')[event.data.stream_id] = next
      return
    }
    case 'PodLogsChunk': {
      const prev = d.read('streams')[event.data.stream_id]
      if (prev?.kind !== 'logs') return
      prev.lines.pushAll(event.data.lines)
      const next: LogStreamState = {
        ...prev,
        rev: prev.rev + 1,
        // Lines prove the follow is live even if `PodLogsOpened` was missed.
        status: prev.status === 'starting' ? 'streaming' : prev.status,
      }
      d.edit('streams')[event.data.stream_id] = next
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
      // A client-chosen id is always a log stream: missing means already stopped.
      if (!prev && isClientStreamId(event.data.stream_id)) return
      if (!prev) {
        // An exec session that failed to attach (e.g. permission denied, no usable shell)
        // never gets a `PodExecReady`, so this is the first time its `stream_id` is seen —
        // seed an errored `kind: 'exec'` entry so PodTerminalTab can surface FR-013's
        // verbatim reason. Log streams are seeded by `startPodLogs` and use client ids,
        // handled above.
        d.edit('streams')[event.data.stream_id] = {
          kind: 'exec',
          status: 'error',
          reason: event.data.reason,
          chunks: new RingBuffer<string>(),
          rev: 0,
        }
        return
      }
      // A container follow (048) runs on the same stream registry as a pod's, so its close
      // arrives here too and reads the same way: a reason of `error`, or any error text, is a
      // failure; anything else is an ordinary end.
      if (prev.kind === 'logs' || prev.kind === 'container-logs') {
        d.edit('streams')[event.data.stream_id] = {
          ...prev,
          status: event.data.reason === 'error' || event.data.error ? 'error' : 'closed',
          reason: event.data.reason,
          error: event.data.error ?? undefined,
        }
        return
      }
      // A container session is not on that registry: it closes with its own
      // `ContainerSessionClosed`, handled in `handlers/containers.ts`.
      if (prev.kind === 'container-session') return
      d.edit('streams')[event.data.stream_id] = {
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
      // An apply may have changed RBAC: cached capability answers are re-asked (033 T031).
      d.effects.push(resetCapabilities)
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
    // 030-kind-cluster-provisioning.
    case 'KindPrerequisitesUpdated': {
      d.edit('kind').prerequisites = event.data.prerequisites
      return
    }
    case 'KindClustersUpdated': {
      d.edit('kind').clusters = event.data.clusters
      return
    }
    case 'KindProvisionProgress': {
      const kind = d.edit('kind')
      const prev = kind.operations[event.data.request_id]
      if (!prev || prev.status !== 'in-progress') return
      kind.operations = {
        ...kind.operations,
        [event.data.request_id]: { ...prev, stage: event.data.stage },
      }
      return
    }
    case 'KindClusterActionFinished': {
      const kind = d.edit('kind')
      const prev = kind.operations[event.data.request_id]
      if (!prev) return
      // A delete only removes the row on the backend's confirmation (Principle I): the
      // authoritative `KindClustersUpdated` that precedes this event already dropped it, but
      // filter again in case ordering differs.
      if (prev.kind === 'delete' && event.data.ok && kind.clusters) {
        kind.clusters = kind.clusters.filter((c) => c.name !== prev.clusterName)
      }
      const cancelled = !event.data.ok && event.data.error === 'cancelled'
      // First run: a freshly created cluster becomes the active context when nothing usable was
      // connected (no kubeconfig, no context, or an unreachable one), so "Create a local
      // cluster" ends on a working cluster instead of the same empty state.
      if (prev.kind === 'create' && event.data.ok) {
        const session = d.read('session')
        const active = session.activeContext
        // Whether anything usable was connected before this create. `ContextsUpdated` (sent
        // just before this event) may already have made the new context active, but the backend
        // has not connected to it: that counts as unusable too.
        const target = `kind-${prev.clusterName}`
        const unusable =
          active === undefined ||
          active === target ||
          session.kubeconfigIssue !== undefined ||
          session.connectionProblems[active] !== undefined
        if (unusable) {
          d.effects.push(() => {
            const s = useStore.getState()
            if (s.session.activeContext === target) s.retryConnection()
            else s.selectContext(target)
          })
        }
      }
      kind.operations = {
        ...kind.operations,
        [event.data.request_id]: {
          ...prev,
          status: event.data.ok ? 'complete' : cancelled ? 'cancelled' : 'failed',
          error: cancelled ? undefined : (event.data.error ?? undefined),
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

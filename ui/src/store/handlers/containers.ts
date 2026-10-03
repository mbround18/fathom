/**
 * 048-docker-management: the local container runtime's events.
 *
 * Split out of `stream.ts` rather than added to its switch because this family answers a
 * different question from every other stream arm — not "what is this cluster doing" but "what is
 * this computer running" — and because the gate the backend applies in one place
 * (`src/backend/cmd/container_runtime.rs`) deserves one place to land on this side too.
 *
 * Presence, inventory, adoption and the cluster links are reduced here, and so are the US5
 * streams: a container's log batches and its session's output land on the `streams` record under
 * the id this client chose, beside pod logs and pod exec. US6's usage ticks land too, bounded on
 * arrival — but only while a panel is asking for them: `startContainerUsage` clears the slice, so
 * figures from containers nobody is looking at cannot linger where a stale one would hide.
 *
 * The three operation arms (US2) land against the one in-flight `operation` and only when the
 * `request_id` still matches: a reply for a preview the operator has already dismissed must not
 * resurrect it.
 */
import { USAGE_HISTORY } from '@/features/containers/usage'
import type { StreamEvent } from '@/protocol/types'

import type { Draft } from '../draft'

/** Whether `handleContainers` owns this event. */
export function isContainerEvent(event: StreamEvent): boolean {
  switch (event.type) {
    case 'ContainerRuntimePresenceUpdated':
    case 'ContainerInventoryUpdated':
    case 'ContainerInventoryUnavailable':
    case 'ContainerAdoptionUpdated':
    case 'ContainerClusterLinksUpdated':
    case 'ContainerOperationPreviewed':
    case 'ContainerOperationFinished':
    case 'ContainerOperationRefused':
    case 'ContainerLogBatch':
    case 'ContainerSessionOutput':
    case 'ContainerSessionClosed':
    case 'ContainerUsageSampled':
      return true
    default:
      return false
  }
}

export function handleContainers(d: Draft, event: StreamEvent): void {
  switch (event.type) {
    // The one event that decides whether the scene exists at all (FR-002). `probed` flips here
    // and only here: the navigator asks once per session, and an answer — including `Absent` —
    // is final until the operator asks again.
    case 'ContainerRuntimePresenceUpdated': {
      const rt = d.edit('containerRuntime')
      rt.presence = event.data.presence
      rt.probed = true
      return
    }
    case 'ContainerInventoryUpdated': {
      const rt = d.edit('containerRuntime')
      rt.inventory = event.data
      // A fresh inventory is the only thing that clears staleness: the operator saw the list go
      // stale, and nothing but new facts may un-say that.
      rt.stale = undefined
      return
    }
    // FR-003: the runtime going away mid-scene marks the list stale rather than blanking it, so
    // the last known state stays readable next to the reason it stopped being true.
    case 'ContainerInventoryUnavailable': {
      d.edit('containerRuntime').stale = event.data.reason
      return
    }
    case 'ContainerAdoptionUpdated': {
      const rt = d.edit('containerRuntime')
      rt.adopted = event.data.adopted
      // `fell_back` is the operator's only signal that the refusals they are about to see come
      // from a grant record Fathom could not read, so it is kept rather than logged.
      rt.adoptionFellBack = event.data.fell_back
      return
    }
    case 'ContainerClusterLinksUpdated': {
      d.edit('containerRuntime').clusterLinks = event.data.links
      return
    }
    // FR-018: the argv arrives here and is displayed verbatim. It is never re-derived on this
    // side, so what the operator reads is what `IssueContainerOperation` will run.
    case 'ContainerOperationPreviewed': {
      const rt = d.edit('containerRuntime')
      if (rt.operation?.requestId !== event.data.request_id) return
      rt.operation = { ...rt.operation, phase: 'previewed', preview: event.data.preview }
      return
    }
    // FR-021: the state change happens here and nowhere earlier. The per-object outcomes are
    // stored whole — an aggregate verdict would hide which member of a project failed (FR-022).
    case 'ContainerOperationFinished': {
      const rt = d.edit('containerRuntime')
      if (rt.operation?.requestId !== event.data.request_id) return
      rt.operation = { ...rt.operation, phase: 'finished', outcome: event.data.outcome }
      return
    }
    // A refusal is the operator's answer, not a log line (FR-014): it is kept on the operation so
    // the control that asked can say what was refused and why.
    case 'ContainerOperationRefused': {
      const rt = d.edit('containerRuntime')
      if (rt.operation?.requestId !== event.data.request_id) return
      rt.operation = { ...rt.operation, phase: 'refused', refusal: event.data.reason }
      return
    }
    // US5 FR-035: a batch of one container's output. An id this client did not start — a late
    // batch after a stop, or another tab's stream on a shared backend — is ignored rather than
    // seeding an entry nobody will ever clean up.
    case 'ContainerLogBatch': {
      const prev = d.read('streams')[event.data.stream_id]
      if (prev?.kind !== 'container-logs') return
      prev.lines.pushAll(event.data.lines)
      d.edit('streams')[event.data.stream_id] = {
        ...prev,
        rev: prev.rev + 1,
        // Once admitted, never un-admitted: a later batch that dropped nothing does not undo the
        // gap an earlier one left.
        truncated: prev.truncated || event.data.truncated,
        // Output proves the follow is live even if nothing else said so.
        status: prev.status === 'starting' ? 'streaming' : prev.status,
      }
      return
    }
    // US5 FR-036: terminal bytes from the session. Plain text, not base64: the pty's bytes are
    // already a string by the time the backend emits them.
    case 'ContainerSessionOutput': {
      const prev = d.read('streams')[event.data.stream_id]
      if (prev?.kind !== 'container-session') return
      prev.chunks.push(event.data.data)
      d.edit('streams')[event.data.stream_id] = {
        ...prev,
        rev: prev.rev + 1,
        status: prev.status === 'attaching' ? 'active' : prev.status,
      }
      return
    }
    // The session is over, or was refused before it began. `error` is the backend's own sentence
    // — a refusal (FR-013), or the runtime's words about a shell it could not start (FR-004) —
    // and it is shown as it was said.
    case 'ContainerSessionClosed': {
      const prev = d.read('streams')[event.data.stream_id]
      if (prev?.kind !== 'container-session') return
      d.edit('streams')[event.data.stream_id] = {
        ...prev,
        status: event.data.error == null ? 'ended' : 'error',
        reason: event.data.error ?? undefined,
      }
      return
    }
    // US6 FR-037/FR-038: one tick. The history is bounded here, on arrival, and `at_cap` is kept
    // rather than inferred from the length — the backend's own history is what fills up, and only
    // it knows when it started dropping (FR-038's visible bound).
    case 'ContainerUsageSampled': {
      const rt = d.edit('containerRuntime')
      const latest = { ...rt.usage.latest }
      const history = { ...rt.usage.history }
      for (const sample of event.data.samples) {
        latest[sample.container_id] = sample
        const kept = [...(history[sample.container_id] ?? []), sample]
        history[sample.container_id] = kept.slice(-USAGE_HISTORY)
      }
      rt.usage = {
        latest,
        history,
        // Once the bound has been hit it stays said: a later tick does not un-drop what was
        // dropped.
        atCap: rt.usage.atCap || event.data.at_cap,
        ticks: rt.usage.ticks + 1,
      }
      return
    }
    default:
      return
  }
}

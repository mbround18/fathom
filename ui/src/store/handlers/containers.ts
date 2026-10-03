/**
 * 048-docker-management: the local container runtime's events.
 *
 * Split out of `stream.ts` rather than added to its switch because this family answers a
 * different question from every other stream arm — not "what is this cluster doing" but "what is
 * this computer running" — and because the gate the backend applies in one place
 * (`src/backend/cmd/container_runtime.rs`) deserves one place to land on this side too.
 *
 * Only the facts Phase 2 and US1 need are reduced here: presence, inventory, adoption and the
 * cluster links. Previews, outcomes, usage samples, log batches and session output arrive on the
 * same channel and are deliberately left to the tasks that render them (T040-T045, T056-T068),
 * because a slice that stored a preview nobody displays would be the first place a stale one
 * could hide.
 */
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
    default:
      return
  }
}

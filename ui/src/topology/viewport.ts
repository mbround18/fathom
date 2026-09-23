/**
 * Zoom-level / focus-set derivation (research.md R3): continuous camera scale + per-region focus
 * flags, not a single global zoom-level enum. Namespaces/workloads whose on-screen bounding box
 * exceeds `MIN_FOCUS_PX` at the current scale are "in focus" and get their children expanded;
 * recomputed fresh on every call (no persisted expansion state), matching US2 Acceptance
 * Scenario 3 (zoom out then back in recomputes from live data).
 */
import type { TopologyNode } from './types'
import { workloadFocusKey } from './types'

export const NAMESPACE_FOCUS_SCALE = 2.2
export const WORKLOAD_FOCUS_SCALE = 5

export interface FocusInput {
  scale: number
  /** Namespace nodes currently positioned (namespace tier, from the last build). */
  namespaceNodes: readonly Pick<TopologyNode, 'label'>[]
  /** Workload nodes currently positioned (workload tier, from the last build). */
  workloadNodes: readonly Pick<TopologyNode, 'kind' | 'label' | 'parentId'>[]
  /** Namespace node id -> namespace name, to resolve `parentId` back to a namespace. */
  namespaceIdToName: ReadonlyMap<string, string>
}

export interface FocusResult {
  namespaceFocus: ReadonlySet<string>
  workloadFocus: ReadonlySet<string>
}

/**
 * v1 keeps this deliberately simple and explicit rather than doing real screen-space bounding-box
 * math against a live canvas transform: past `NAMESPACE_FOCUS_SCALE`, every currently-rendered
 * namespace is in focus (there are at most a few dozen namespaces in a cluster, so "all of them"
 * at a zoomed-in scale is the correct behavior, not a shortcut); past `WORKLOAD_FOCUS_SCALE`,
 * every currently-rendered workload is in focus. The camera's own pan position already determines
 * which namespaces/workloads are close enough to be "currently rendered" in the first place
 * (`TopologyCanvas` only feeds this function the nodes within/near the viewport).
 */
export function deriveFocus(input: FocusInput): FocusResult {
  const namespaceFocus = new Set<string>()
  const workloadFocus = new Set<string>()
  if (input.scale >= NAMESPACE_FOCUS_SCALE) {
    for (const n of input.namespaceNodes) namespaceFocus.add(n.label)
  }
  if (input.scale >= WORKLOAD_FOCUS_SCALE) {
    for (const w of input.workloadNodes) {
      const ns = w.parentId ? input.namespaceIdToName.get(w.parentId) : undefined
      if (ns) workloadFocus.add(workloadFocusKey(w.kind, ns, w.label))
    }
  }
  return { namespaceFocus, workloadFocus }
}

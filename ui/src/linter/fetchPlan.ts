/**
 * Which per-object detail reads the linter needs, and how to throttle them (spec FR-002, research
 * R6). Pure: the panel performs the sends via the store's existing read-only `inspect` action.
 */
import type { DetailTarget, LintInput } from './types'
import { detailKeyOf, readyRows, SECTIONS, targetsFor, WORKLOADS } from './util'

export const MAX_IN_FLIGHT = 8
export const REQUEST_EXPIRY_MS = 10_000

export function detailTargets(input: LintInput): DetailTarget[] {
  const out: DetailTarget[] = []
  const add = (kind: string, section: string, minDesired?: number) => {
    const rows = readyRows(input, section)
    if (!rows) return
    out.push(
      ...targetsFor(
        kind,
        rows.filter((r) => minDesired === undefined || (r.desired ?? 0) >= minDesired),
      ),
    )
  }
  add('Pod', SECTIONS.pods)
  for (const w of WORKLOADS) add(w.kind, w.section)
  add('ReplicaSet', SECTIONS.replicasets, 1)
  add('Role', SECTIONS.roles)
  add('ClusterRole', SECTIONS.clusterRoles)
  add('PodDisruptionBudget', SECTIONS.pdbs)
  return out
}

/**
 * Targets to send now. `sent` maps a target key to the time it was sent and is never cleared
 * within a context, so an expired (lost or forbidden) read is not retried in a loop.
 */
export function nextBatch(
  targets: readonly DetailTarget[],
  hasDetail: (t: DetailTarget) => boolean,
  sent: ReadonlyMap<string, number>,
  now: number,
  limit = MAX_IN_FLIGHT,
): DetailTarget[] {
  let active = 0
  const unsent: DetailTarget[] = []
  for (const t of targets) {
    if (hasDetail(t)) continue
    const at = sent.get(detailKeyOf(t))
    if (at === undefined) unsent.push(t)
    else if (now - at <= REQUEST_EXPIRY_MS) active++
  }
  return unsent.slice(0, Math.max(0, limit - active))
}

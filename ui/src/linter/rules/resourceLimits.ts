/** Workload containers (and naked pods) missing CPU/memory requests and/or limits. */
import type { Finding, Rule } from '../types'
import {
  containersOf,
  isObj,
  makeFinding,
  nameOf,
  missingReason,
  outcome,
  ownerRefs,
  parseRaw,
  podSpecOf,
  readyRows,
  ref,
  SECTIONS,
  skipped,
  WORKLOADS,
  type Obj,
} from '../util'

function missingFor(container: Obj): string[] {
  const res = isObj(container.resources) ? container.resources : {}
  const req = isObj(res.requests) ? res.requests : {}
  const lim = isObj(res.limits) ? res.limits : {}
  const out: string[] = []
  if (req.cpu === undefined) out.push('cpu request')
  if (req.memory === undefined) out.push('memory request')
  if (lim.cpu === undefined) out.push('cpu limit')
  if (lim.memory === undefined) out.push('memory limit')
  return out
}

export const resourceLimits: Rule = {
  id: 'resource-limits',
  category: 'resource-limits',
  severity: 'medium',
  title: 'Missing resource requests/limits',
  run(input) {
    const findings: Finding[] = []
    let checked = 0
    let pending = 0
    const missing: string[] = []
    const targets: { kind: string; section: string }[] = [
      ...WORKLOADS,
      { kind: 'Pod', section: SECTIONS.pods },
    ]
    let available = 0
    for (const { kind, section } of targets) {
      const rows = readyRows(input, section)
      if (!rows) {
        missing.push(section)
        continue
      }
      available++
      for (const row of rows) {
        const d = input.detail(kind, row.namespace, row.name)
        const raw = d ? parseRaw(d.data) : undefined
        if (!raw) {
          pending++
          continue
        }
        // Pods owned by a controller are judged through the controller (no duplicate noise).
        if (kind === 'Pod' && ownerRefs(raw).length > 0) continue
        checked++
        const perContainer: string[] = []
        for (const c of containersOf(podSpecOf(kind, raw))) {
          const m = missingFor(c)
          if (m.length > 0) perContainer.push(`${nameOf(c)}: ${m.join(', ')}`)
        }
        if (perContainer.length > 0) {
          findings.push(
            makeFinding(
              this,
              ref(section, kind, row.namespace, row.name),
              `Missing resources (${perContainer.join('; ')})`,
            ),
          )
        }
      }
    }
    if (available === 0) return skipped(this, missingReason(missing))
    return outcome(this, findings, checked, pending, missing.length ? missingReason(missing) : undefined)
  },
}

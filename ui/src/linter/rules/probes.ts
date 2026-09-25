/** Long-running workload containers missing liveness/readiness (or any) probes. */
import type { Finding, Rule } from '../types'
import {
  containersOf,
  makeFinding,
  nameOf,
  missingReason,
  outcome,
  parseRaw,
  podSpecOf,
  readyRows,
  ref,
  skipped,
  WORKLOADS,
} from '../util'

export const probes: Rule = {
  id: 'probes',
  category: 'probes',
  severity: 'medium',
  title: 'Missing liveness/readiness/startup probes',
  run(input) {
    const findings: Finding[] = []
    let checked = 0
    let pending = 0
    const missing: string[] = []
    let available = 0
    for (const { kind, section } of WORKLOADS) {
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
        checked++
        const bad: string[] = []
        for (const c of containersOf(podSpecOf(kind, raw))) {
          const lacks: string[] = []
          if (!c.livenessProbe) lacks.push('liveness')
          if (!c.readinessProbe) lacks.push('readiness')
          if (lacks.length === 0) continue
          if (lacks.length === 2 && !c.startupProbe) lacks.push('startup')
          bad.push(`${nameOf(c)}: ${lacks.join(', ')}`)
        }
        if (bad.length > 0) {
          findings.push(
            makeFinding(this, ref(section, kind, row.namespace, row.name), `Missing probes (${bad.join('; ')})`),
          )
        }
      }
    }
    if (available === 0) return skipped(this, missingReason(missing))
    return outcome(this, findings, checked, pending, missing.length ? missingReason(missing) : undefined)
  },
}

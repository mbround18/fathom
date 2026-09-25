/** Objects whose managedFields record a write via a deprecated/removed apiVersion. */
import { deprecationFor } from '../deprecatedApiTable'
import type { Finding, Rule } from '../types'
import {
  arr,
  isObj,
  makeFinding,
  metaOf,
  outcome,
  parseRaw,
  readyRows,
  ref,
  SECTIONS,
  skipped,
  WORKLOADS,
} from '../util'

const SOURCES = [
  ...WORKLOADS,
  { kind: 'Pod', section: SECTIONS.pods },
  { kind: 'ReplicaSet', section: SECTIONS.replicasets },
  { kind: 'Role', section: SECTIONS.roles },
  { kind: 'ClusterRole', section: SECTIONS.clusterRoles },
  { kind: 'PodDisruptionBudget', section: SECTIONS.pdbs },
] as const

export const deprecatedApi: Rule = {
  id: 'deprecated-api',
  category: 'api-deprecation',
  severity: 'medium',
  title: 'Deprecated or removed API versions',
  run(input) {
    const findings: Finding[] = []
    let checked = 0
    let pending = 0
    let available = 0
    for (const { kind, section } of SOURCES) {
      const rows = readyRows(input, section)
      if (!rows) continue
      available++
      for (const row of rows) {
        // ReplicaSets with no replicas are historical rollout leftovers; not worth a read.
        if (kind === 'ReplicaSet' && (row.desired ?? 0) === 0) continue
        const d = input.detail(kind, row.namespace, row.name)
        const raw = d ? parseRaw(d.data) : undefined
        if (!raw) {
          pending++
          continue
        }
        checked++
        const hits = new Map<string, string>()
        for (const mf of arr(metaOf(raw).managedFields)) {
          if (!isObj(mf) || typeof mf.apiVersion !== 'string') continue
          const dep = deprecationFor(mf.apiVersion, kind)
          if (dep) hits.set(dep.apiVersion, `${dep.apiVersion} (removed in ${dep.removedIn}; use ${dep.replacement})`)
        }
        if (hits.size > 0) {
          findings.push(
            makeFinding(this, ref(section, kind, row.namespace, row.name), `Written via deprecated API: ${[...hits.values()].join(', ')}`),
          )
        }
      }
    }
    if (available === 0) return skipped(this, 'No resource lists are readable')
    return outcome(this, findings, checked, pending)
  },
}

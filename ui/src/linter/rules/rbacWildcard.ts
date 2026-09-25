/** Roles/ClusterRoles granting wildcard verbs or resources. Built-in `system:*` roles and cluster-admin are exempt. */
import type { Finding, Rule } from '../types'
import {
  arr,
  isObj,
  makeFinding,
  missingReason,
  outcome,
  readyRows,
  ref,
  SECTIONS,
  skipped,
} from '../util'

function isBuiltIn(name: string): boolean {
  return name.startsWith('system:') || name === 'cluster-admin'
}

function wildcardParts(data: unknown): string[] {
  if (!isObj(data)) return []
  const parts = new Set<string>()
  for (const r of arr(data.rules)) {
    if (!isObj(r)) continue
    if (arr(r.verbs).includes('*')) parts.add('verbs')
    if (arr(r.resources).includes('*')) parts.add('resources')
  }
  return [...parts]
}

export const rbacWildcard: Rule = {
  id: 'rbac-wildcard',
  category: 'rbac',
  severity: 'high',
  title: 'RBAC wildcard verbs/resources',
  run(input) {
    const findings: Finding[] = []
    let checked = 0
    let pending = 0
    const missing: string[] = []
    const targets = [
      { kind: 'Role', section: SECTIONS.roles },
      { kind: 'ClusterRole', section: SECTIONS.clusterRoles },
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
        if (isBuiltIn(row.name)) continue
        const d = input.detail(kind, row.namespace, row.name)
        if (!d) {
          pending++
          continue
        }
        checked++
        const parts = wildcardParts(d.data)
        if (parts.length > 0) {
          findings.push(
            makeFinding(
              this,
              ref(section, kind, row.namespace, row.name),
              `Grants wildcard (*) ${parts.join(' and ')}`,
            ),
          )
        }
      }
    }
    if (available === 0) return skipped(this, missingReason(missing))
    return outcome(
      this,
      findings,
      checked,
      pending,
      missing.length ? missingReason(missing) : undefined,
    )
  },
}

/** Multi-replica Deployments/StatefulSets/standalone ReplicaSets with no matching PodDisruptionBudget. */
import type { Finding, Rule } from '../types'
import {
  arr,
  isObj,
  makeFinding,
  metaOf,
  missingReason,
  outcome,
  ownerRefs,
  parseRaw,
  readyRows,
  ref,
  SECTIONS,
  skipped,
  type Obj,
} from '../util'

function strMap(v: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  if (isObj(v)) for (const [k, val] of Object.entries(v)) if (typeof val === 'string') out[k] = val
  return out
}

/** Pod-template labels of a workload/ReplicaSet, falling back to its selector's matchLabels. */
export function templateLabels(raw: Obj): Record<string, string> {
  const spec = isObj(raw.spec) ? raw.spec : {}
  const tpl = isObj(spec.template) ? spec.template : {}
  const fromTpl = strMap(metaOf(tpl).labels)
  if (Object.keys(fromTpl).length > 0) return fromTpl
  const sel = isObj(spec.selector) ? spec.selector : {}
  return strMap(sel.matchLabels)
}

/** Does the PDB's selector cover pods carrying `labels`? Expression selectors match conservatively. */
export function pdbMatches(pdbRaw: Obj, labels: Record<string, string>): boolean {
  const spec = isObj(pdbRaw.spec) ? pdbRaw.spec : {}
  const sel = isObj(spec.selector) ? spec.selector : undefined
  if (!sel) return true
  const ml = strMap(sel.matchLabels)
  const hasExpr = arr(sel.matchExpressions).length > 0
  if (hasExpr) return true
  const entries = Object.entries(ml)
  if (entries.length === 0) return true
  return entries.every(([k, v]) => labels[k] === v)
}

const CANDIDATES = [
  { kind: 'Deployment', section: SECTIONS.deployments },
  { kind: 'StatefulSet', section: SECTIONS.statefulsets },
  { kind: 'ReplicaSet', section: SECTIONS.replicasets },
] as const

export const missingPdb: Rule = {
  id: 'missing-pdb',
  category: 'pdb',
  severity: 'medium',
  title: 'Multi-replica workloads without a PodDisruptionBudget',
  run(input) {
    const pdbRows = readyRows(input, SECTIONS.pdbs)
    if (!pdbRows) return skipped(this, missingReason([SECTIONS.pdbs]))
    let pending = 0
    const pdbs: { namespace: string | null; raw: Obj }[] = []
    for (const p of pdbRows) {
      const d = input.detail('PodDisruptionBudget', p.namespace, p.name)
      const raw = d ? parseRaw(d.data) : undefined
      if (!raw) pending++
      else pdbs.push({ namespace: p.namespace, raw })
    }
    const pdbPending = pending
    const findings: Finding[] = []
    let checked = 0
    const missing: string[] = []
    for (const { kind, section } of CANDIDATES) {
      const rows = readyRows(input, section)
      if (!rows) {
        missing.push(section)
        continue
      }
      for (const row of rows) {
        if ((row.desired ?? 0) <= 1) continue
        const d = input.detail(kind, row.namespace, row.name)
        const raw = d ? parseRaw(d.data) : undefined
        if (!raw) {
          pending++
          continue
        }
        if (kind === 'ReplicaSet' && ownerRefs(raw).some((o) => o.kind === 'Deployment')) continue
        checked++
        const labels = templateLabels(raw)
        const covered = pdbs.some((p) => p.namespace === row.namespace && pdbMatches(p.raw, labels))
        // PDB details still loading: hold the verdict rather than report a false violation.
        if (!covered && pdbPending === 0) {
          findings.push(
            makeFinding(
              this,
              ref(section, kind, row.namespace, row.name),
              `Runs ${String(row.desired)} replicas but no PodDisruptionBudget selects it`,
            ),
          )
        }
      }
    }
    return outcome(this, findings, checked, pending, missing.length ? missingReason(missing) : undefined)
  },
}

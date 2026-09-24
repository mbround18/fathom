/** Shared helpers for linter rules: section constants, safe JSON access, list/detail lookups. */
import { SECTION } from '@/protocol/commands'
import type { ObjectRef } from '@/resources/types'

import type { DetailTarget, Finding, LintInput, RuleOutcome, Rule } from './types'

export const SECTIONS = {
  pods: SECTION.pods,
  deployments: SECTION.deployments,
  statefulsets: SECTION.statefulsets,
  daemonsets: SECTION.daemonsets,
  replicasets: SECTION.replicasets,
  configmaps: SECTION.configmaps,
  secrets: SECTION.secrets,
  pdbs: SECTION.poddisruptionbudgets,
  roles: SECTION.roles,
  clusterRoles: SECTION.clusterRoles,
} as const

/** Workload kinds owning a pod template (spec Edge Cases: attribution target). */
export const WORKLOADS = [
  { kind: 'Deployment', section: SECTIONS.deployments },
  { kind: 'StatefulSet', section: SECTIONS.statefulsets },
  { kind: 'DaemonSet', section: SECTIONS.daemonsets },
] as const

export type Obj = Record<string, unknown>

export function isObj(v: unknown): v is Obj {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

export function arr(v: unknown): unknown[] {
  return Array.isArray(v) ? v : []
}

const rawCache = new WeakMap<object, Obj | null>()

/** Parsed `raw_json` of a `*Details` payload, or `undefined` when absent/malformed. */
export function parseRaw(data: unknown): Obj | undefined {
  if (!isObj(data)) return undefined
  const hit = rawCache.get(data)
  if (hit !== undefined) return hit ?? undefined
  let out: Obj | null = null
  const raw = data.raw_json
  if (typeof raw === 'string') {
    try {
      const parsed: unknown = JSON.parse(raw)
      if (isObj(parsed)) out = parsed
    } catch {
      out = null
    }
  }
  rawCache.set(data, out)
  return out ?? undefined
}

export interface NamedRow {
  name: string
  namespace: string | null
  desired?: number
}

/** Rows of a ready list section as `{name, namespace}`; `undefined` when the section is unavailable. */
export function readyRows(input: LintInput, section: string): NamedRow[] | undefined {
  const snap = input.rows(section)
  if (!snap || snap.status !== 'ready') return undefined
  const out: NamedRow[] = []
  for (const r of snap.rows) {
    if (!isObj(r) || typeof r.name !== 'string') continue
    const row: NamedRow = {
      name: r.name,
      namespace: typeof r.namespace === 'string' ? r.namespace : null,
    }
    if (typeof r.desired === 'number') row.desired = r.desired
    out.push(row)
  }
  return out
}

export function metaOf(raw: Obj): Obj {
  return isObj(raw.metadata) ? raw.metadata : {}
}

export function ownerRefs(raw: Obj): Obj[] {
  return arr(metaOf(raw).ownerReferences).filter(isObj)
}

/** The pod spec of a Pod or of a workload's pod template. */
export function podSpecOf(kind: string, raw: Obj): Obj | undefined {
  const spec = isObj(raw.spec) ? raw.spec : undefined
  if (!spec) return undefined
  if (kind === 'Pod') return spec
  const tpl = isObj(spec.template) ? spec.template : undefined
  return tpl && isObj(tpl.spec) ? tpl.spec : undefined
}

export function containersOf(podSpec: Obj | undefined): Obj[] {
  return arr(podSpec?.containers).filter(isObj)
}

export function ref(section: string, kind: string, namespace: string | null, name: string): ObjectRef {
  return { section, kind, namespace, name }
}

export function makeFinding(
  rule: Rule,
  target: ObjectRef,
  message: string,
  discriminator = '',
): Finding {
  return {
    id: `${rule.id}|${target.kind}|${target.namespace ?? '*'}|${target.name}|${discriminator}`,
    rule: rule.id,
    category: rule.category,
    severity: rule.severity,
    message,
    ref: target,
  }
}

export function skipped(rule: Rule, reason: string): RuleOutcome {
  return {
    rule: rule.id,
    category: rule.category,
    title: rule.title,
    status: 'skipped',
    reason,
    checked: 0,
    flagged: 0,
    pending: 0,
    findings: [],
  }
}

export function outcome(
  rule: Rule,
  findings: Finding[],
  checked: number,
  pending: number,
  reason?: string,
): RuleOutcome {
  const flagged = new Set(findings.map((f) => `${f.ref.kind}|${f.ref.namespace}|${f.ref.name}`)).size
  return {
    rule: rule.id,
    category: rule.category,
    title: rule.title,
    status: pending > 0 ? 'pending' : 'evaluated',
    ...(reason !== undefined ? { reason } : {}),
    checked,
    flagged,
    pending,
    findings,
  }
}

/** Human list of section labels that could not be read, e.g. `pods, secrets`. */
export function missingReason(missing: string[]): string {
  return `No visibility into: ${missing.map((s) => s.split('/').pop()).join(', ')}`
}

export function detailKeyOf(t: DetailTarget): string {
  return `${t.kind}|${t.namespace ?? '*'}|${t.name}`
}

/** All objects a rule reads details for, given the rows of one section. */
export function targetsFor(kind: string, rows: NamedRow[]): DetailTarget[] {
  return rows.map((r) => ({ kind, namespace: r.namespace, name: r.name }))
}

export function nameOf(c: Obj): string {
  return typeof c.name === 'string' ? c.name : '?'
}

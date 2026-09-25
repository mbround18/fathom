/**
 * Cluster linter types (017-cluster-linter, data-model.md). Pure data: no React, no store.
 */
import type { ObjectRef } from '@/resources/types'

export type Severity = 'high' | 'medium' | 'low'

export type Category =
  'resource-limits' | 'probes' | 'api-deprecation' | 'unused-object' | 'naked-pod' | 'rbac' | 'pdb'

export type RuleId =
  | 'resource-limits'
  | 'probes'
  | 'deprecated-api'
  | 'unused-config'
  | 'unused-serviceaccount'
  | 'naked-pod'
  | 'rbac-wildcard'
  | 'missing-pdb'

export interface Finding {
  id: string
  rule: RuleId
  category: Category
  severity: Severity
  message: string
  ref: ObjectRef
}

export type RuleStatus = 'evaluated' | 'pending' | 'skipped'

export interface RuleResult {
  rule: RuleId
  category: Category
  title: string
  status: RuleStatus
  /** Why the rule was skipped, or which data it could not see (FR-010). */
  reason?: string
  /** Objects examined; a rule with `checked === 0` is excluded from the score. */
  checked: number
  flagged: number
  /** Objects whose per-object details have not arrived yet. */
  pending: number
}

export interface RuleOutcome extends RuleResult {
  findings: Finding[]
}

export interface LintReport {
  findings: Finding[]
  rules: RuleResult[]
  counts: Record<Severity, number>
  /** 0-100, or null when nothing was checked. */
  score: number | null
  pendingObjects: number
}

export interface ListSnapshot {
  status: 'idle' | 'loading' | 'ready' | 'error'
  rows: readonly unknown[]
}

/** The slice of the store the linter reads, decoupled for unit testing (no Zustand). */
export interface LintInput {
  rows(section: string): ListSnapshot | undefined
  /** `data` is the typed `*Details` payload (carrying `raw_json`). */
  detail(kind: string, namespace: string | null, name: string): { data: unknown } | undefined
}

export interface Rule {
  id: RuleId
  category: Category
  severity: Severity
  title: string
  run(input: LintInput): RuleOutcome
}

export interface DetailTarget {
  kind: string
  namespace: string | null
  name: string
}

export const SEVERITIES: readonly Severity[] = ['high', 'medium', 'low']

export const CATEGORY_LABEL: Record<Category, string> = {
  'resource-limits': 'Resources',
  probes: 'Probes',
  'api-deprecation': 'Deprecated API',
  'unused-object': 'Unused',
  'naked-pod': 'Naked pods',
  rbac: 'RBAC',
  pdb: 'PDB',
}

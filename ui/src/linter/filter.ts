import type { Category, Finding, Severity } from './types'

export interface FindingFilter {
  severity: Severity | null
  category: Category | null
}

export function filterFindings(findings: readonly Finding[], f: FindingFilter): Finding[] {
  return findings.filter(
    (x) =>
      (f.severity === null || x.severity === f.severity) &&
      (f.category === null || x.category === f.category),
  )
}

/** store -> LintReport (contracts/linter-rules-module.md). Pure; never throws, never dispatches. */
import { RULES } from './rules'
import type { Finding, LintInput, LintReport, RuleResult, Severity } from './types'

export function evaluate(input: LintInput): LintReport {
  const findings: Finding[] = []
  const rules: RuleResult[] = []
  for (const rule of RULES) {
    let out
    try {
      out = rule.run(input)
    } catch {
      out = {
        rule: rule.id,
        category: rule.category,
        title: rule.title,
        status: 'skipped' as const,
        reason: 'Rule could not be evaluated',
        checked: 0,
        flagged: 0,
        pending: 0,
        findings: [],
      }
    }
    const { findings: f, ...result } = out
    findings.push(...f)
    rules.push(result)
  }

  const counts: Record<Severity, number> = { high: 0, medium: 0, low: 0 }
  for (const f of findings) counts[f.severity]++

  // Rules with nothing to check (or skipped) are excluded from the denominator (spec Edge Cases).
  let checked = 0
  let flagged = 0
  let pendingObjects = 0
  for (const r of rules) {
    pendingObjects += r.pending
    if (r.status === 'skipped' || r.checked === 0) continue
    checked += r.checked
    flagged += r.flagged
  }
  const score = checked === 0 ? null : Math.round((100 * (checked - flagged)) / checked)

  const order: Record<Severity, number> = { high: 0, medium: 1, low: 2 }
  findings.sort((a, b) => order[a.severity] - order[b.severity] || a.id.localeCompare(b.id))
  return { findings, rules, counts, score, pendingObjects }
}

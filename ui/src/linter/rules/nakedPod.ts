/** Pods with no ownerReferences (not created by any controller). */
import type { Finding, Rule } from '../types'
import { makeFinding, missingReason, outcome, ownerRefs, parseRaw, readyRows, ref, SECTIONS, skipped } from '../util'

export const nakedPod: Rule = {
  id: 'naked-pod',
  category: 'naked-pod',
  severity: 'high',
  title: 'Naked pods (no owning controller)',
  run(input) {
    const rows = readyRows(input, SECTIONS.pods)
    if (!rows) return skipped(this, missingReason([SECTIONS.pods]))
    const findings: Finding[] = []
    let checked = 0
    let pending = 0
    for (const row of rows) {
      const d = input.detail('Pod', row.namespace, row.name)
      const raw = d ? parseRaw(d.data) : undefined
      if (!raw) {
        pending++
        continue
      }
      checked++
      if (ownerRefs(raw).length === 0) {
        findings.push(
          makeFinding(
            this,
            ref(SECTIONS.pods, 'Pod', row.namespace, row.name),
            'Pod has no owning controller; it will not be rescheduled if it or its node dies',
          ),
        )
      }
    }
    return outcome(this, findings, checked, pending)
  },
}

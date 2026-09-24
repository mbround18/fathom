/**
 * ServiceAccounts have no backend capability (research.md R3), so this rule always reports itself
 * as skipped through the same FR-010 path as any other missing-visibility rule.
 */
import type { Rule } from '../types'
import { skipped } from '../util'

export const unusedServiceAccount: Rule = {
  id: 'unused-serviceaccount',
  category: 'unused-object',
  severity: 'low',
  title: 'Unused ServiceAccounts',
  run() {
    return skipped(this, 'ServiceAccounts are not readable by Fathom yet')
  },
}

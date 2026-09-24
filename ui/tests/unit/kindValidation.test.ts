import { describe, expect, it } from 'vitest'

import { validateKindClusterName } from '@/components/panels/kind-validation'

describe('validateKindClusterName', () => {
  it('accepts DNS-1123 labels', () => {
    for (const n of ['a', 'dev-1', '0abc', 'a'.repeat(63)])
      expect(validateKindClusterName(n, [])).toBeNull()
  })
  it('rejects unsafe names', () => {
    for (const n of ['', 'Dev', '-a', 'a-', 'a_b', 'a b', '$(x)', 'a'.repeat(64)])
      expect(validateKindClusterName(n, [])).not.toBeNull()
  })
  it('rejects collisions', () => {
    expect(validateKindClusterName('dev', ['dev'])).toContain('already exists')
  })
})

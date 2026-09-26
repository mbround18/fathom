import { describe, expect, it } from 'vitest'

import { descriptorFor } from '@/resources'
import { NAV_GROUPS, PANEL_SECTIONS } from '@/resources/nav'

/**
 * TODO by batch: still without a descriptor. Each batch REMOVES its keys here when it registers
 * the descriptor (this test fails if a listed key already resolves, so the list cannot go stale).
 */
const PENDING: Record<string, string[]> = {
  A: [],
  B: [],
  C: [],
  D: [],
}
const PENDING_ALL = new Set(Object.values(PENDING).flat())

const KEYS = NAV_GROUPS.flatMap((g) => g.sections.map((s) => s.key))

describe('nav section coverage', () => {
  it.each(KEYS)('%s resolves to a descriptor, panel, or documented gap', (key) => {
    const resolved = descriptorFor(key) !== undefined || PANEL_SECTIONS.has(key)
    if (PENDING_ALL.has(key)) {
      expect(resolved, `${key} now resolves; remove it from the allowlist`).toBe(false)
    } else {
      expect(resolved, `${key} has no descriptor, panel, or allowlist entry`).toBe(true)
    }
  })
})

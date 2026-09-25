import { describe, expect, it } from 'vitest'

import { descriptorFor } from '@/resources'
import { NAV_GROUPS } from '@/resources/nav'

/** Sections PaneHost renders directly (panel components), no descriptor needed. */
const PANEL_SECTIONS = new Set([
  'overview',
  'events',
  'integrations/metrics',
  'integrations/grafana',
  'integrations/argo',
  'integrations/flux',
  'custom-resources',
  'network/portforwards',
  'integrations/gatekeeper',
  'integrations/cluster-settings',
  'team/teams',
  'team/profiles',
  'audit-log',
  'access/namespace-access',
  'access/oidc-config',
  'integrations/vulnerabilities',
])

/** No backend command exists at all (documented unavailable). */
const UNAVAILABLE = new Set(['config/verticalpodautoscalers'])

/**
 * TODO by batch: still without a descriptor. Each batch REMOVES its keys here when it registers
 * the descriptor (this test fails if a listed key already resolves, so the list cannot go stale).
 */
const PENDING: Record<string, string[]> = {
  A: [],
  B: [],
  C: [],
  D: [
    'config/horizontalpodautoscalers',
    'config/priorityclasses',
    'config/runtimeclasses',
    'config/leases',
    'config/mutatingwebhooks',
    'config/validatingwebhooks',
  ],
}
const PENDING_ALL = new Set(Object.values(PENDING).flat())

const KEYS = NAV_GROUPS.flatMap((g) => g.sections.map((s) => s.key))

describe('nav section coverage', () => {
  it.each(KEYS)('%s resolves to a descriptor, panel, or documented gap', (key) => {
    const resolved = descriptorFor(key) !== undefined || PANEL_SECTIONS.has(key)
    if (PENDING_ALL.has(key) || UNAVAILABLE.has(key)) {
      expect(resolved, `${key} now resolves; remove it from the allowlist`).toBe(false)
    } else {
      expect(resolved, `${key} has no descriptor, panel, or allowlist entry`).toBe(true)
    }
  })
})

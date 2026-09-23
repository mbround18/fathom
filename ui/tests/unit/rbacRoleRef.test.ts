/**
 * 011-rbac-oidc-administration US1 (T017): `isDanglingRoleRef` — a RoleBinding's `roleRef` may
 * resolve against either a Role (its own namespace) or a ClusterRole; a ClusterRoleBinding's may
 * only ever resolve against a ClusterRole.
 *
 * NOTE: placed under `tests/unit/` (not `src/resources/__tests__/`) to match this repo's vitest
 * `include` (`vitest.config.ts`: `tests/unit/**\/*.test.{ts,tsx}`) and every other resource test
 * in the codebase (e.g. `tests/unit/descriptors.test.ts`) — see final report for this deviation
 * from the literal path suggested in the task brief.
 */
import { describe, expect, it } from 'vitest'

import { isDanglingRoleRef } from '@/resources/rbacRoleRef'

describe('isDanglingRoleRef', () => {
  const roleNames = new Set(['pod-reader'])
  const clusterRoleNames = new Set(['view', 'edit'])

  it('resolves when roleRef matches a Role name (RoleBinding case)', () => {
    expect(isDanglingRoleRef('pod-reader', { roleNames, clusterRoleNames })).toBe(false)
  })

  it('resolves when roleRef matches a ClusterRole name (RoleBinding case)', () => {
    expect(isDanglingRoleRef('view', { roleNames, clusterRoleNames })).toBe(false)
  })

  it('is dangling when roleRef matches neither set', () => {
    expect(isDanglingRoleRef('nonexistent', { roleNames, clusterRoleNames })).toBe(true)
  })

  it('ClusterRoleBinding case: only checks against ClusterRoles, even if a same-named Role exists', () => {
    // No `roleNames` passed at all (a ClusterRoleBinding's roleRef can never target a Role).
    expect(isDanglingRoleRef('pod-reader', { clusterRoleNames })).toBe(true)
    expect(isDanglingRoleRef('view', { clusterRoleNames })).toBe(false)
  })

  it('is dangling against empty sets', () => {
    expect(
      isDanglingRoleRef('anything', { roleNames: new Set(), clusterRoleNames: new Set() }),
    ).toBe(true)
  })
})

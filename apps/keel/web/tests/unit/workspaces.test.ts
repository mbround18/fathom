/**
 * Spec 063 T055: who the web offers an action to, matching the server's `Capability::of`
 * (keel-core authz). Operate is View + Operate; Administer needs a workspace admin.
 */
import { describe, expect, it } from 'vitest'

import { canAdminister, canOperate } from '@/features/hosts/workspaces'
import type { Me, TeamRole } from '@/protocol/wire'

const WS = 'ws-1'

function me(instance_role: Me['instance_role'], role?: TeamRole, workspace = WS): Me {
  const grants = role ? [{ workspace, workspace_slug: 'ops', role, profiles: [] }] : []
  return { instance_role, grants } as unknown as Me
}

describe('canAdminister', () => {
  it('lets an instance owner or admin administer any workspace', () => {
    expect(canAdminister(me('owner'), WS)).toBe(true)
    expect(canAdminister(me('admin'), WS)).toBe(true)
  })

  it('lets a workspace admin administer that workspace only', () => {
    expect(canAdminister(me('user', 'ws_admin'), WS)).toBe(true)
    expect(canAdminister(me('user', 'ws_admin', 'ws-2'), WS)).toBe(false)
  })

  it('refuses every other role', () => {
    for (const role of ['viewer', 'operator', 'approver'] as const) {
      expect(canAdminister(me('user', role), WS)).toBe(false)
    }
    expect(canAdminister(me('user'), WS)).toBe(false)
    expect(canAdminister(null, WS)).toBe(false)
  })

  it('is narrower than operating: an operator operates but does not administer', () => {
    expect(canOperate(me('user', 'operator'), WS)).toBe(true)
    expect(canAdminister(me('user', 'operator'), WS)).toBe(false)
  })
})

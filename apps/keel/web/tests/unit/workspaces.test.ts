/**
 * Spec 063 T055: who the web offers an action to, matching the server's `Capability::of`
 * (keel-core authz). Operate is View + Operate; Administer needs a workspace admin. Spec 064 T009:
 * Approve comes with an approver or workspace admin grant on the whole workspace.
 */
import { describe, expect, it } from 'vitest'

import { canAdminister, canApprove, canOperate } from '@/features/hosts/workspaces'
import type { Me, TeamRole } from '@/protocol/wire'

const WS = 'ws-1'

function me(instance_role: Me['instance_role'], role?: TeamRole, workspace = WS, profiles: string[] = []): Me {
  const grants = role ? [{ workspace, workspace_slug: 'ops', role, profiles }] : []
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

describe('canApprove', () => {
  it('lets an instance owner or admin approve in any workspace', () => {
    expect(canApprove(me('owner'), WS)).toBe(true)
    expect(canApprove(me('admin'), WS)).toBe(true)
  })

  it('lets an approver or workspace admin approve in that workspace only', () => {
    expect(canApprove(me('user', 'approver'), WS)).toBe(true)
    expect(canApprove(me('user', 'ws_admin'), WS)).toBe(true)
    expect(canApprove(me('user', 'approver', 'ws-2'), WS)).toBe(false)
  })

  it('refuses a grant scoped to profiles: it covers hosts, not the workspace', () => {
    expect(canApprove(me('user', 'approver', WS, ['web']), WS)).toBe(false)
  })

  it('refuses viewers, operators and nobody', () => {
    expect(canApprove(me('user', 'viewer'), WS)).toBe(false)
    expect(canApprove(me('user', 'operator'), WS)).toBe(false)
    expect(canApprove(me('user'), WS)).toBe(false)
    expect(canApprove(null, WS)).toBe(false)
  })
})

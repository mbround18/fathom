import { describe, expect, it } from 'vitest'

import { canSeeSettings, resolveTab, showsSettings, visibleTabs } from '@/features/settings/tabs'
import type { Me, MeGrant } from '@/protocol/wire'

const ALL = ['workspace', 'enrollment', 'tokens', 'identity', 'system']

function user(grants: Partial<MeGrant>[]): Me {
  return {
    id: 'u1',
    display_name: 'Ana',
    email: null,
    instance_role: 'user',
    grants: grants.map((g): MeGrant => ({ workspace: 'ws1', workspace_slug: 'ws1', role: 'viewer', profiles: [], ...g })),
    verification: { verified: false, at: null, window_secs: 300, enrolled: false, required: false, recovering: false },
    keel_version: null,
    update_dot_dismissed: null,
  }
}

describe('settings tabs', () => {
  it('owners see every tab, Workspace first and System last', () => {
    expect(visibleTabs('owner')).toEqual(ALL)
  })
  it('admins see every tab, System included (spec 061 FR-024)', () => {
    expect(visibleTabs('admin')).toEqual(ALL)
  })
  it('users and nobody see none, and no nav entry', () => {
    expect(visibleTabs('user')).toEqual([])
    expect(visibleTabs(null)).toEqual([])
    expect(canSeeSettings('user')).toBe(false)
    expect(canSeeSettings('admin')).toBe(true)
    expect(canSeeSettings('owner')).toBe(true)
    expect(showsSettings('user', user([]), 'ws1')).toBe(false)
  })
  it('a ws_admin of the current workspace sees Workspace alone, and the nav entry (spec 066 R10)', () => {
    const me = user([{ workspace: 'ws1', role: 'ws_admin' }])
    expect(visibleTabs('user', me, 'ws1')).toEqual(['workspace'])
    expect(showsSettings('user', me, 'ws1')).toBe(true)
    expect(visibleTabs('user', me, 'ws2')).toEqual([])
    expect(showsSettings('user', me, null)).toBe(false)
    expect(visibleTabs('user', user([{ workspace: 'ws1', role: 'operator' }]), 'ws1')).toEqual([])
  })
  it('an unknown or forbidden tab goes to Workspace', () => {
    expect(resolveTab('system', 'owner')).toBe('system')
    expect(resolveTab('system', 'admin')).toBe('system')
    expect(resolveTab('system', 'user')).toBe('workspace')
    expect(resolveTab('tokens', 'admin')).toBe('tokens')
    expect(resolveTab('nope', 'owner')).toBe('workspace')
    expect(resolveTab(undefined, 'owner')).toBe('workspace')
    const me = user([{ workspace: 'ws1', role: 'ws_admin' }])
    expect(resolveTab('tokens', 'user', me, 'ws1')).toBe('workspace')
  })
})

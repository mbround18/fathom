import { describe, expect, it } from 'vitest'

import { canSeeSettings, resolveTab, visibleTabs } from '@/features/settings/tabs'

describe('settings tabs', () => {
  it('owners see every tab, System last', () => {
    expect(visibleTabs('owner')).toEqual(['enrollment', 'tokens', 'identity', 'system'])
  })
  it('admins see every tab but System', () => {
    expect(visibleTabs('admin')).toEqual(['enrollment', 'tokens', 'identity'])
  })
  it('users and nobody see none, and no nav entry', () => {
    expect(visibleTabs('user')).toEqual([])
    expect(visibleTabs(null)).toEqual([])
    expect(canSeeSettings('user')).toBe(false)
    expect(canSeeSettings('admin')).toBe(true)
    expect(canSeeSettings('owner')).toBe(true)
  })
  it('an unknown or forbidden tab goes to Enrollment', () => {
    expect(resolveTab('system', 'owner')).toBe('system')
    expect(resolveTab('system', 'admin')).toBe('enrollment')
    expect(resolveTab('tokens', 'admin')).toBe('tokens')
    expect(resolveTab('nope', 'owner')).toBe('enrollment')
    expect(resolveTab(undefined, 'owner')).toBe('enrollment')
  })
})

/** 033 US3: advisory profile filter and the Teams state reducer. */
import { describe, expect, it } from 'vitest'

import { filterProfiles, isProfileVisible } from '@/features/teams/filter'
import { initialTeamsState, reduceTeams } from '@/features/teams/state'
import type { TeamLink } from '@/protocol/types'

const links: TeamLink[] = [
  { group: 'fathom-editors', profile_id: 'p-edit' },
  { group: 'fathom-viewers', profile_id: 'p-view' },
  { group: 'fathom-editors', profile_id: 'p-view' },
]

describe('advisory profile filter', () => {
  it('shows unlinked profiles to everyone', () => {
    expect(isProfileVisible('p-open', links, [])).toBe(true)
  })
  it('shows linked profiles only to members of a linked group', () => {
    expect(isProfileVisible('p-edit', links, ['fathom-viewers'])).toBe(false)
    expect(isProfileVisible('p-edit', links, ['fathom-editors'])).toBe(true)
    expect(isProfileVisible('p-view', links, ['fathom-viewers'])).toBe(true)
    expect(isProfileVisible('p-view', links, [])).toBe(false)
  })
  it('filters a list', () => {
    const ps = [{ id: 'p-open' }, { id: 'p-edit' }, { id: 'p-view' }]
    expect(filterProfiles(ps, links, ['fathom-viewers']).map((p) => p.id)).toEqual([
      'p-open',
      'p-view',
    ])
  })
})

describe('teams reducer', () => {
  it('keeps directory, key status and links', () => {
    const s = initialTeamsState()
    const snap = {
      issuer_id: 'rauthy',
      users: [],
      groups: [{ name: 'fathom-editors', member_count: 1 }],
      fetched_at: 5,
      stale: false,
      error: null,
    }
    expect(reduceTeams(s, { type: 'IdpDirectoryUpdated', data: snap })).toBe(true)
    expect(
      reduceTeams(s, {
        type: 'IdpAdminKeyStatus',
        data: { issuer_id: 'rauthy', present: true },
      }),
    ).toBe(true)
    expect(reduceTeams(s, { type: 'TeamLinksUpdated', data: { links } })).toBe(true)
    expect(s.directory.rauthy?.groups[0]?.name).toBe('fathom-editors')
    expect(s.adminKey.rauthy).toBe(true)
    expect(s.links).toHaveLength(3)
    expect(reduceTeams(s, { type: 'BackendStarted' } as never)).toBe(false)
  })

  it('a failed refresh snapshot replaces with stale + error, data retained by the backend', () => {
    const s = initialTeamsState()
    const base = {
      issuer_id: 'rauthy',
      users: [{ sub: 'u', username: 'alice', email: null, groups: [], enabled: true }],
      groups: [],
      fetched_at: 5,
      stale: true,
      error: 'IdP unreachable',
    }
    reduceTeams(s, { type: 'IdpDirectoryUpdated', data: base })
    expect(s.directory.rauthy?.users).toHaveLength(1)
    expect(s.directory.rauthy?.stale).toBe(true)
  })
})

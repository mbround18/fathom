import { describe, expect, it } from 'vitest'

import {
  applySuggestion,
  ceilingText,
  changeOf,
  outsideCeiling,
  settled,
  stepLine,
  suggestionCount,
  suggestionReason,
} from '../../src/features/hosts/profiles'
import type { ProfileSuggestion } from '../../src/protocol/wire'

const s = (over: Partial<ProfileSuggestion>): ProfileSuggestion => ({
  host_id: 'h',
  profile: 'k3s-agent',
  action: 'add',
  groups: ['k3s_agents'],
  first_seen_at: '2026-10-27T12:00:00Z',
  ...over,
})

describe('host profiles', () => {
  it('finds profiles outside the ceiling', () => {
    expect(outsideCeiling({ profiles: ['base', 'web'], allowed_profiles: ['base'] })).toEqual(['web'])
    expect(outsideCeiling({ profiles: ['base'], allowed_profiles: ['base'] })).toEqual([])
  })

  it('diffs a draft against the current profiles', () => {
    expect(changeOf(['base', 'web'], ['base', 'debian'])).toEqual({ add: ['debian'], remove: ['web'] })
    expect(changeOf(['base'], ['base'])).toEqual({ add: [], remove: [] })
  })

  it('applies a suggestion without duplicating', () => {
    expect(applySuggestion(['base'], s({}))).toEqual(['base', 'k3s-agent'])
    expect(applySuggestion(['base', 'k3s-agent'], s({}))).toEqual(['base', 'k3s-agent'])
    expect(applySuggestion(['base', 'k3s-agent'], s({ action: 'remove' }))).toEqual(['base'])
  })

  it('words the reasons, the ceiling and the count', () => {
    expect(suggestionReason(s({ groups: ['a', 'b'] }))).toBe('from a, b')
    expect(suggestionReason(s({ action: 'remove', groups: [] }))).toBe('no group maps to it')
    expect(ceilingText([])).toBe('Its runner has not reported which profiles it accepts.')
    expect(ceilingText(['base', 'web'])).toBe('This host accepts: base, web.')
    expect(suggestionCount(2)).toBe('2 suggested from inventory groups')
  })

  it('lines a planned step', () => {
    const step = { position: 1, job: 'debian', via: ['keel-node', 'debian'], params: {}, credential_names: [] }
    expect(stepLine(step)).toBe('debian (from keel-node → debian)')
    expect(stepLine({ ...step, via: [] })).toBe('debian')
  })

  it('settles suggestions a change answers', () => {
    const items = [s({}), s({ profile: 'web', action: 'remove' })]
    expect(settled(items, ['base', 'k3s-agent', 'web'])).toEqual([items[1]])
    expect(settled(items, ['base'])).toEqual([items[0]])
  })
})

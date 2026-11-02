import { describe, expect, it } from 'vitest'
import { emptyVariant } from '@companion/ui/kit'

describe('emptyVariant (spec 063 R3)', () => {
  it('says nothing matches when a filter emptied the list, whoever is looking', () => {
    expect(emptyVariant({ filtered: true, canAct: true })).toBe('no-match')
    expect(emptyVariant({ filtered: true, canAct: false })).toBe('no-match')
  })

  it('guides someone who can take the first step', () => {
    expect(emptyVariant({ filtered: false, canAct: true })).toBe('guided')
  })

  it('names who can act for someone who cannot', () => {
    expect(emptyVariant({ filtered: false, canAct: false })).toBe('restricted')
  })
})

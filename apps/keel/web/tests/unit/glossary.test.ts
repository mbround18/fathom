/**
 * The glossary's shape (spec 063 T035, FR-020, FR-024): every required term is there, each entry
 * is short enough for a toggletip and in sentence case, and every guide it names exists.
 */
import { describe, expect, it } from 'vitest'

import { GLOSSARY, type TermId } from '@/help/glossary'

import { resolveGuideRef } from './support/guides'

/** FR-020's terms, as `TermId`s. */
const REQUIRED: readonly TermId[] = [
  'binding',
  'receiver',
  'trusted-signers',
  'early-check',
  'review',
  'base-profile',
  'enrolment-token',
  'fingerprint',
  'preview-credential',
  'inventory-managed',
  'inventory-repository',
  'workspace',
  'team',
  'grant',
  'profile',
]

const entries = Object.entries(GLOSSARY) as [TermId, (typeof GLOSSARY)[TermId]][]

/** Sentence case: a capital first letter, then lower case (acronyms such as SSH aside). */
function sentenceCase(s: string): boolean {
  const [first, ...rest] = s.split(' ')
  if (first === undefined || !/^[A-Z]/.test(first)) return false
  return rest.every((w) => w === w.toLowerCase() || w === w.toUpperCase())
}

describe('glossary (spec 063 FR-020)', () => {
  it('defines every required term', () => {
    expect(REQUIRED.filter((t) => !(t in GLOSSARY))).toEqual([])
  })

  it.each(entries)('%s: at most two sentences and 240 characters', (_id, e) => {
    expect(e.text.length).toBeLessThanOrEqual(240)
    expect(e.text.trim().split('. ').length).toBeLessThanOrEqual(2)
    expect(e.text).toMatch(/[.]$/)
  })

  it.each(entries)('%s: term and text in sentence case', (_id, e) => {
    expect(sentenceCase(e.term)).toBe(true)
    expect(e.text).toMatch(/^[A-Z]/)
  })

  it.each(entries)('%s: its guide resolves', (_id, e) => {
    if (e.guide !== undefined) expect(resolveGuideRef(e.guide)).toBeNull()
  })

  it('names each term once', () => {
    const terms = entries.map(([, e]) => e.term.toLowerCase())
    expect(new Set(terms).size).toBe(terms.length)
  })
})

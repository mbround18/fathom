import { describe, expect, it } from 'vitest'
import { slug, slugger } from '@companion/ui/kit'

describe('slug (spec 063 R7)', () => {
  it('lower-cases and hyphenates a heading', () => {
    expect(slug('Enroll a host')).toBe('enroll-a-host')
  })

  it('strips punctuation', () => {
    expect(slug("What's a binding?")).toBe('whats-a-binding')
    expect(slug('Approve or deny a change (with a passkey)')).toBe('approve-or-deny-a-change-with-a-passkey')
  })

  it('collapses repeated spaces and hyphens', () => {
    expect(slug('Managed  and   repository')).toBe('managed-and-repository')
    expect(slug('Hosts -- the basics')).toBe('hosts-the-basics')
    expect(slug('Before - after')).toBe('before-after')
  })

  it('keeps digits and trims the ends', () => {
    expect(slug('  Step 2: run it  ')).toBe('step-2-run-it')
  })
})

describe('slugger (one per page)', () => {
  it('numbers repeated headings from 2', () => {
    const next = slugger()
    expect(next('x')).toBe('x')
    expect(next('x')).toBe('x-2')
    expect(next('X')).toBe('x-3')
    expect(next('y')).toBe('y')
  })

  it('starts afresh for each page', () => {
    const a = slugger()
    a('x')
    expect(slugger()('x')).toBe('x')
  })
})

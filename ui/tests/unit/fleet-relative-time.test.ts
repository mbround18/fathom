import { describe, expect, it } from 'vitest'

import { formatUpdatedAgo } from '@/components/fleet/relative-time'

const T = '2026-09-23T00:00:00Z'
const at = (s: number) => Date.parse(T) + s * 1000

describe('formatUpdatedAgo', () => {
  it('formats seconds, minutes, hours', () => {
    expect(formatUpdatedAgo(T, at(0))).toBe('updated 0s ago')
    expect(formatUpdatedAgo(T, at(12.9))).toBe('updated 12s ago')
    expect(formatUpdatedAgo(T, at(120))).toBe('updated 2m ago')
    expect(formatUpdatedAgo(T, at(7300))).toBe('updated 2h ago')
  })
  it('clamps future timestamps to 0s', () => {
    expect(formatUpdatedAgo(T, at(-5))).toBe('updated 0s ago')
  })
  it('returns null for empty or invalid input', () => {
    expect(formatUpdatedAgo('', at(0))).toBeNull()
    expect(formatUpdatedAgo('nope', at(0))).toBeNull()
  })
})

import { describe, expect, it } from 'vitest'

import { formatExpiryCountdown } from '@/lib/cert-expiry'

describe('formatExpiryCountdown', () => {
  it('formats positive values as days remaining', () => {
    expect(formatExpiryCountdown(10 * 86400 + 5)).toBe('expires in 10d')
  })
  it('formats negative values as expired, never a bare negative', () => {
    expect(formatExpiryCountdown(-3 * 86400)).toBe('expired 3d ago')
  })
  it('formats zero as expires in 0d', () => {
    expect(formatExpiryCountdown(0)).toBe('expires in 0d')
  })
  it('formats null as unknown', () => {
    expect(formatExpiryCountdown(null)).toBe('unknown')
  })
})

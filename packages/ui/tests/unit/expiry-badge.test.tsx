import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { ExpiryBadge, expiryTone } from '../../src/kit'

const NOW = Date.parse('2026-11-02T12:00:00Z')
const days = (n: number) => NOW + n * 86_400_000

describe('expiryTone', () => {
  it.each([
    [90, 'ok'],
    [30.5, 'ok'],
    [30, 'soon'],
    [21, 'soon'],
    [7, 'urgent'],
    [0.01, 'urgent'],
    [0, 'expired'],
    [-3, 'expired'],
  ] as const)('%d days left is %s', (left, tone) => {
    expect(expiryTone(days(left), NOW)).toBe(tone)
  })

  it('takes its own window, and is null for an unreadable time', () => {
    expect(expiryTone(days(50), NOW, { soonDays: 60 })).toBe('soon')
    expect(expiryTone(days(10), NOW, { urgentDays: 14 })).toBe('urgent')
    expect(expiryTone('nope', NOW)).toBeNull()
  })
})

describe('ExpiryBadge', () => {
  it('says when in words, with the exact time in the title', () => {
    render(<ExpiryBadge notAfter={new Date(days(12)).toISOString()} now={NOW} />)
    const text = screen.getByText('Expires in 12 days')
    expect(text.tagName).toBe('TIME')
    const badge = text.closest('[data-expiry]')
    expect(badge).toHaveAttribute('data-expiry', 'soon')
    expect(badge).toHaveAttribute('title', '2026-11-14 12:00 UTC')
  })

  it('says Expired once past, and takes its own words', () => {
    render(
      <>
        <ExpiryBadge notAfter={days(-2)} now={NOW} />
        <ExpiryBadge notAfter={days(200)} now={NOW} label="Valid until" pastLabel="Lapsed" />
      </>,
    )
    expect(screen.getByText('Expired 2 days ago').closest('[data-expiry]')).toHaveAttribute('data-expiry', 'expired')
    expect(screen.getByText('Valid until in 200 days').closest('[data-expiry]')).toHaveAttribute('data-expiry', 'ok')
  })

  it('says unknown for an unreadable time', () => {
    render(<ExpiryBadge notAfter="nope" now={NOW} />)
    expect(screen.getByText('Expires unknown')).toBeInTheDocument()
  })
})

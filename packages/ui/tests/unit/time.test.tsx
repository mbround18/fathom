import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { formatRelative, formatUtc, Time } from '../../src/kit'

const NOW = Date.parse('2026-11-02T12:00:00Z')

describe('formatRelative', () => {
  it.each([
    ['2026-11-02T11:59:40Z', 'just now'],
    ['2026-11-02T11:55:00Z', '5 minutes ago'],
    ['2026-11-02T13:00:00Z', 'in 1 hour'],
    ['2026-11-03T12:00:00Z', 'tomorrow'],
    ['2026-10-23T12:00:00Z', '10 days ago'],
    ['2027-12-07T12:00:00Z', 'in 400 days'],
  ])('%s reads %s', (at, text) => {
    expect(formatRelative(at, NOW)).toBe(text)
  })

  it('takes numbers and Dates, and is empty for an unreadable time', () => {
    expect(formatRelative(NOW - 120_000, NOW)).toBe('2 minutes ago')
    expect(formatRelative(new Date(NOW + 7_200_000), NOW)).toBe('in 2 hours')
    expect(formatRelative('not a time', NOW)).toBe('')
  })
})

describe('formatUtc', () => {
  it('is the date and minute in UTC', () => {
    expect(formatUtc('2026-11-02T14:05:59+02:00')).toBe('2026-11-02 12:05 UTC')
    expect(formatUtc('nope')).toBe('')
  })
})

describe('Time', () => {
  it('shows UTC by default, relative on request, with the exact time in the title', () => {
    render(
      <>
        <Time at="2026-11-02T11:55:00Z" />
        <Time at="2026-11-02T11:55:00Z" relative now={NOW} />
      </>,
    )
    const abs = screen.getByText('2026-11-02 11:55 UTC')
    expect(abs.tagName).toBe('TIME')
    expect(abs).toHaveAttribute('dateTime', '2026-11-02T11:55:00.000Z')
    const rel = screen.getByText('5 minutes ago')
    expect(rel).toHaveAttribute('title', '2026-11-02 11:55 UTC')
  })

  it('says unknown for an unreadable time', () => {
    render(<Time at="garbage" relative />)
    expect(screen.getByText('unknown')).toBeInTheDocument()
  })
})

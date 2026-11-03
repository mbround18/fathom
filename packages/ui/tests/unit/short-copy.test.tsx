import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { Fingerprint, ShortCopy, shorten } from '../../src/kit'

describe('shorten', () => {
  it.each([
    ['sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08', 'sha256:9f86d081…b0f00a08'],
    ['SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s', 'SHA256:uNiVztks…Cvj3tD2s'],
    ['AB:CD:EF:01:23:45:67:89:AB:CD:EF', 'AB:CD:EF…AB:CD:EF'],
    ['short', 'short'],
    ['0123456789abcdefg', '0123456789abcdefg'],
  ])('%s → %s', (value, short) => {
    expect(shorten(value)).toBe(short)
  })

  it('keeps as many characters from each end as asked', () => {
    expect(shorten('0123456789abcdef0123', 4)).toBe('0123…0123')
  })
})

describe('ShortCopy', () => {
  it('shows the short form, keeps the full value in the title, and copies the full value', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    Object.defineProperty(globalThis.navigator, 'clipboard', { value: { writeText }, configurable: true })
    const full = 'sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08'
    render(<ShortCopy label="digest" value={full} />)
    expect(screen.getByText('sha256:9f86d081…b0f00a08')).toHaveAttribute('title', full)
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy full digest' }))
      await Promise.resolve()
    })
    expect(writeText).toHaveBeenCalledWith(full)
    expect(screen.getByRole('status')).toHaveTextContent('Full digest copied')
  })

  it('takes a short form of its own, and Fingerprint labels itself', () => {
    render(
      <>
        <ShortCopy label="revision" value="0a1b2c3d4e5f6a7b8c9d" short="0a1b2c3" />
        <Fingerprint value="SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s" />
      </>,
    )
    expect(screen.getByText('0a1b2c3')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Copy full fingerprint' })).toBeInTheDocument()
  })
})

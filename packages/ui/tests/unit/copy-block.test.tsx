import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { copyText, CopyBlock } from '../../src/kit'

function stubClipboard(writeText: (t: string) => Promise<void>) {
  Object.defineProperty(globalThis.navigator, 'clipboard', { value: { writeText }, configurable: true })
}

afterEach(() => {
  vi.useRealTimers()
})

describe('copyText', () => {
  it('is true once the clipboard has the text', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    stubClipboard(writeText)
    expect(await copyText('abc')).toBe(true)
    expect(writeText).toHaveBeenCalledWith('abc')
  })

  it('is false when there is no clipboard', async () => {
    stubClipboard(() => Promise.reject(new Error('denied')))
    expect(await copyText('abc')).toBe(false)
  })
})

describe('CopyBlock', () => {
  it('labels the value and copies it, saying Copied for a moment', async () => {
    vi.useFakeTimers()
    const writeText = vi.fn(() => Promise.resolve())
    stubClipboard(writeText)
    render(<CopyBlock label="Widget address" value="widgets.example.test" data="address" />)
    const value = document.querySelector('[data-copy="address"]')
    expect(value).toHaveAccessibleName('Widget address')
    expect(value).toHaveAttribute('data-masked', 'false')

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy widget address' }))
      await Promise.resolve()
    })
    expect(writeText).toHaveBeenCalledWith('widgets.example.test')
    expect(screen.getByRole('button', { name: 'Copy widget address' })).toHaveTextContent('Copied')
    expect(screen.getByRole('status')).toHaveTextContent('Widget address copied')
    act(() => {
      vi.advanceTimersByTime(2000)
    })
    expect(screen.getByRole('button', { name: 'Copy widget address' })).toHaveTextContent('Copy')
  })

  it('masks the value until Reveal, and copies the real value either way', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    stubClipboard(writeText)
    render(<CopyBlock label="Join token" value="secret-token" masked data="token" />)
    const value = document.querySelector('[data-copy="token"]')
    expect(value).toHaveAttribute('data-masked', 'true')
    expect(value).not.toHaveTextContent('secret-token')
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Copy join token' }))
      await Promise.resolve()
    })
    expect(writeText).toHaveBeenCalledWith('secret-token')

    fireEvent.click(screen.getByRole('button', { name: 'Reveal join token' }))
    expect(value).toHaveTextContent('secret-token')
    expect(screen.getByRole('button', { name: 'Hide join token' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('caps the dots at 32, however long the value', () => {
    render(<CopyBlock label="Key" value={'x'.repeat(100)} masked data="key" />)
    expect(document.querySelector('[data-copy="key"]')?.textContent).toHaveLength(32)
  })
})

import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { SecretInput, SecretValue } from '../../src/kit'

afterEach(() => {
  vi.useRealTimers()
})

const SECRET = 'hunter2-but-longer-and-very-secret'

async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name }))
    await Promise.resolve()
  })
}

describe('SecretValue', () => {
  it('keeps the value out of the page until Reveal, then hides it again after a while', async () => {
    vi.useFakeTimers()
    const reveal = vi.fn(() => Promise.resolve(SECRET))
    render(<SecretValue label="Database password" reveal={reveal} hideAfterMs={5000} data="db" />)
    expect(document.body).not.toHaveTextContent(SECRET)
    const value = document.querySelector('[data-secret="db"]')
    expect(value).toHaveAccessibleName('Database password')
    expect(value).toHaveAttribute('data-revealed', 'false')
    expect(reveal).not.toHaveBeenCalled()

    await click('Reveal database password')
    expect(reveal).toHaveBeenCalledOnce()
    expect(value).toHaveTextContent(SECRET)
    expect(screen.getByRole('status')).toHaveTextContent('Database password shown; it hides after 5 seconds')

    act(() => {
      vi.advanceTimersByTime(5000)
    })
    expect(document.body).not.toHaveTextContent(SECRET)
    expect(screen.getByRole('button', { name: 'Reveal database password' })).toBeInTheDocument()
  })

  it('Hide takes the value away at once, and Copy copies the value', async () => {
    const writeText = vi.fn(() => Promise.resolve())
    Object.defineProperty(globalThis.navigator, 'clipboard', { value: { writeText }, configurable: true })
    render(<SecretValue label="Token" reveal={() => Promise.resolve(SECRET)} />)
    await click('Reveal token')
    await click('Copy token')
    expect(writeText).toHaveBeenCalledWith(SECRET)
    expect(screen.getByRole('status')).toHaveTextContent('Token copied')
    await click('Hide token')
    expect(document.body).not.toHaveTextContent(SECRET)
  })

  it('stays hidden when the person backs out, and says why when the reveal fails', async () => {
    let answer: Promise<string | null> = Promise.resolve(null)
    render(<SecretValue label="Token" reveal={() => answer} />)
    await click('Reveal token')
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByRole('button', { name: 'Reveal token' })).toBeEnabled()

    answer = Promise.reject(new Error('step-up refused'))
    await click('Reveal token')
    await act(async () => {
      await Promise.resolve()
    })
    expect(screen.getByRole('alert')).toHaveTextContent('Could not reveal the token: step-up refused')
  })

  it('cannot be revealed twice at once', async () => {
    render(<SecretValue label="Token" reveal={() => new Promise<string>(() => undefined)} />)
    await click('Reveal token')
    expect(screen.getByRole('button', { name: 'Reveal token' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Reveal token' })).toHaveTextContent('Revealing…')
  })
})

describe('SecretInput', () => {
  it('hides what is typed until Show, and tells the browser not to keep it', () => {
    render(
      <>
        <label htmlFor="pw">Password</label>
        <SecretInput id="pw" defaultValue={SECRET} />
      </>,
    )
    const input = screen.getByLabelText('Password')
    expect(input).toHaveAttribute('type', 'password')
    expect(input).toHaveAttribute('autocomplete', 'off')
    expect(input).toHaveAttribute('spellcheck', 'false')
    fireEvent.click(screen.getByRole('button', { name: 'Show' }))
    expect(input).toHaveAttribute('type', 'text')
    fireEvent.click(screen.getByRole('button', { name: 'Hide' }))
    expect(input).toHaveAttribute('type', 'password')
  })
})

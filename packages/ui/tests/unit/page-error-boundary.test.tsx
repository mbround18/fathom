import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { PageErrorBoundary } from '../../src/kit'

function Boom({ fail }: { fail: boolean }) {
  if (fail) throw new Error('kaboom')
  return <p>fine</p>
}

function Broken(): never {
  throw new Error('widgets is undefined')
}

function Thrower(): never {
  // Not an Error: the boundary still shows it as a message.
  // eslint-disable-next-line @typescript-eslint/only-throw-error
  throw 'a plain string'
}

describe('PageErrorBoundary', () => {
  // React logs the caught error; keep the run quiet.
  beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
  })
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('shows its children while they render', () => {
    render(
      <PageErrorBoundary>
        <p>Widgets</p>
      </PageErrorBoundary>,
    )
    expect(screen.getByText('Widgets')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('says something went wrong, with the message, as an error pane', () => {
    const { container } = render(
      <PageErrorBoundary>
        <Broken />
      </PageErrorBoundary>,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('Something went wrong showing this page. widgets is undefined')
    expect(container.querySelector('[data-pane-state="error"]')).not.toBeNull()
    expect(container.querySelector('[data-error-message]')).toHaveTextContent('widgets is undefined')
  })

  it('shows a thrown non-Error as its text', () => {
    const { container } = render(
      <PageErrorBoundary>
        <Thrower />
      </PageErrorBoundary>,
    )
    expect(container.querySelector('[data-error-message]')).toHaveTextContent('a plain string')
  })

  it('reloads the page on Reload', async () => {
    const reload = vi.fn()
    vi.stubGlobal('location', { reload })
    render(
      <PageErrorBoundary>
        <Broken />
      </PageErrorBoundary>,
    )
    await userEvent.click(screen.getByRole('button', { name: 'Reload' }))
    expect(reload).toHaveBeenCalledTimes(1)
    vi.unstubAllGlobals()
  })

  it('keeps the error while resetKey stays, and clears it when resetKey changes', () => {
    const { rerender, container } = render(
      <PageErrorBoundary resetKey="a">
        <Boom fail />
      </PageErrorBoundary>,
    )
    expect(container.querySelector('[data-pane-state="error"]')).not.toBeNull()
    expect(screen.getByRole('alert')).toHaveTextContent('kaboom')

    rerender(
      <PageErrorBoundary resetKey="a">
        <Boom fail={false} />
      </PageErrorBoundary>,
    )
    expect(screen.getByRole('alert')).toBeInTheDocument()

    rerender(
      <PageErrorBoundary resetKey="b">
        <Boom fail={false} />
      </PageErrorBoundary>,
    )
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByText('fine')).toBeInTheDocument()
  })

  it('without resetKey stays as it was', () => {
    const { rerender } = render(
      <PageErrorBoundary>
        <Boom fail />
      </PageErrorBoundary>,
    )
    rerender(
      <PageErrorBoundary>
        <Boom fail={false} />
      </PageErrorBoundary>,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('kaboom')
  })
})

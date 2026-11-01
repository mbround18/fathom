import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { PageErrorBoundary } from '@/ui/PageErrorBoundary'

// A page that throws while rendering says so, with the error, instead of blanking the app.

function Broken(): never {
  throw new Error('runs is undefined')
}

beforeEach(() => {
  // React logs the caught error; keep the test output clean.
  vi.spyOn(console, 'error').mockImplementation(() => undefined)
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('PageErrorBoundary', () => {
  it('shows its children while they render', () => {
    render(
      <PageErrorBoundary>
        <p>Runs</p>
      </PageErrorBoundary>,
    )
    expect(screen.getByText('Runs')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('says something went wrong, with the error message, as an error pane', () => {
    const { container } = render(
      <PageErrorBoundary>
        <Broken />
      </PageErrorBoundary>,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('Something went wrong showing this page. runs is undefined')
    expect(container.querySelector('[data-pane-state="error"]')).not.toBeNull()
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
})

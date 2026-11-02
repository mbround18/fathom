import { render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { PageErrorBoundary } from '../../src/kit'

function Boom({ fail }: { fail: boolean }) {
  if (fail) throw new Error('kaboom')
  return <p>fine</p>
}

describe('PageErrorBoundary', () => {
  // React logs the caught error; keep the run quiet.
  beforeEach(() => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
  })
  afterEach(() => {
    vi.restoreAllMocks()
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

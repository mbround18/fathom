import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { LoadState } from '../../src/kit'

const content = <p>Blue widget</p>

describe('LoadState', () => {
  it('shows a status line while loading, not the children', () => {
    render(<LoadState loading>{content}</LoadState>)
    expect(screen.getByRole('status')).toHaveTextContent('Loading…')
    expect(screen.getByRole('status')).toHaveAttribute('data-pane-state', 'loading')
    expect(screen.queryByText('Blue widget')).toBeNull()
  })

  it('uses the caller’s label', () => {
    render(<LoadState loading label="Loading widgets…" />)
    expect(screen.getByRole('status')).toHaveTextContent('Loading widgets…')
  })

  it('lets a failure win over loading, naming what failed', () => {
    render(
      <LoadState loading failed={{ kind: 'error', code: 'internal', message: 'Down.' }} what="widgets">
        {content}
      </LoadState>,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('Failed to load widgets. Down. (internal)')
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('says "this" when the caller names nothing', () => {
    render(<LoadState loading={false} failed={{ kind: 'error', code: 'internal', message: 'Down.' }} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Failed to load this.')
  })

  it('renders the children once loaded', () => {
    render(<LoadState loading={false}>{content}</LoadState>)
    expect(screen.getByText('Blue widget')).toBeInTheDocument()
  })
})

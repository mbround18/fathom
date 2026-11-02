import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { NoMatch } from '../../src/kit'

describe('NoMatch', () => {
  it('says nothing matches, as an empty no-match pane', () => {
    const { container } = render(<NoMatch what="widgets" onClear={vi.fn()} />)
    expect(screen.getByText('No widgets match.')).toBeInTheDocument()
    expect(container.querySelector('[data-pane-state="empty"][data-empty="no-match"]')).not.toBeNull()
  })

  it('uses the caller’s message instead', () => {
    render(<NoMatch what="widgets" message="Nothing matches “purple”." onClear={vi.fn()} />)
    expect(screen.getByText('Nothing matches “purple”.')).toBeInTheDocument()
    expect(screen.queryByText('No widgets match.')).toBeNull()
  })

  it('clears the filter', async () => {
    const clear = vi.fn()
    render(<NoMatch what="widgets" onClear={clear} />)
    await userEvent.click(screen.getByRole('button', { name: 'Clear filter' }))
    expect(clear).toHaveBeenCalledTimes(1)
  })
})

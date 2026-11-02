import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { LoadFailed, type Refusal } from '../../src/kit'

const opError: Refusal = { kind: 'error', code: 'internal', message: 'The widget store is down.' }

describe('LoadFailed', () => {
  it('says what failed and why, as an error pane', () => {
    const { container } = render(<LoadFailed what="widgets" refusal={opError} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Failed to load widgets. The widget store is down. (internal)')
    expect(container.querySelector('[data-pane-state="error"]')).not.toBeNull()
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull()
  })

  it('shows a denial with its action and reason', () => {
    render(<LoadFailed what="widgets" refusal={{ kind: 'denied', action: 'list_widgets', reason: 'no_grant' }} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Failed to load widgets. Denied: list_widgets (no_grant)')
  })

  it('runs the load again on Retry', async () => {
    const retry = vi.fn()
    render(<LoadFailed what="widgets" refusal={opError} onRetry={retry} />)
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }))
    expect(retry).toHaveBeenCalledTimes(1)
  })
})

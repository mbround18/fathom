import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { ErrorLine } from '../../src/kit'

describe('ErrorLine', () => {
  it('renders nothing without a message', () => {
    const { container } = render(<ErrorLine message={null} />)
    expect(container).toBeEmptyDOMElement()
    render(<ErrorLine message="" />)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('is an alert with the message and no button by default', () => {
    render(<ErrorLine message="The name is taken." />)
    expect(screen.getByRole('alert')).toHaveTextContent('The name is taken.')
    expect(screen.queryByRole('button')).toBeNull()
  })

  it('calls onDismiss from Dismiss', async () => {
    const dismiss = vi.fn()
    render(<ErrorLine message="The name is taken." onDismiss={dismiss} />)
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(dismiss).toHaveBeenCalledTimes(1)
  })
})

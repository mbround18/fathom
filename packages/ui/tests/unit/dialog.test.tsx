import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { Dialog } from '../../src/kit'

function Opener({ onClose = vi.fn<() => void>() }: { onClose?: () => void }) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        onClick={() => {
          setOpen(true)
        }}
      >
        Rename widget
      </button>
      <Dialog
        open={open}
        title="Rename widget"
        onClose={() => {
          onClose()
          setOpen(false)
        }}
        footer={<button>Save</button>}
      >
        <input aria-label="Name" />
      </Dialog>
    </>
  )
}

describe('Dialog', () => {
  it('renders nothing while closed', () => {
    render(<Dialog open={false} onClose={vi.fn()} title="Rename widget" />)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('is named by its title and described by its description, marked data-summary', () => {
    render(<Dialog open onClose={vi.fn()} title="Rename widget" description="Shown everywhere." data-x="1" />)
    const dialog = screen.getByRole('dialog', { name: 'Rename widget' })
    expect(dialog).toHaveAccessibleDescription('Shown everywhere.')
    expect(dialog.querySelector('[data-summary]')).toHaveTextContent('Shown everywhere.')
    expect(dialog).toHaveAttribute('data-x', '1')
  })

  it('without a description, is described by its title, with no data-summary', () => {
    render(<Dialog open onClose={vi.fn()} title="Rename widget" />)
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveAccessibleDescription('Rename widget')
    expect(dialog.querySelector('[data-summary]')).toBeNull()
  })

  it('calls onClose on Escape, and gives focus back to what opened it', async () => {
    const onClose = vi.fn()
    const user = userEvent.setup()
    render(<Opener onClose={onClose} />)
    const trigger = screen.getByRole('button', { name: 'Rename widget' })
    await user.click(trigger)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    await user.keyboard('{Escape}')
    expect(onClose).toHaveBeenCalledTimes(1)
    await vi.waitFor(() => {
      expect(trigger).toHaveFocus()
    })
  })

  it('calls onClose from the close button', async () => {
    const onClose = vi.fn()
    const user = userEvent.setup()
    render(<Opener onClose={onClose} />)
    await user.click(screen.getByRole('button', { name: 'Rename widget' }))
    await user.click(screen.getByRole('button', { name: 'Close' }))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})

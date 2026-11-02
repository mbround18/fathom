import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { ConfirmDialog, PreviewDetails, type Preview } from '../../src/kit'

const preview: Preview = { summary: 'Remove Blue widget.', affected: [], step_up: false }

describe('PreviewDetails', () => {
  it('shows nothing for a plain preview', () => {
    const { container } = render(<PreviewDetails preview={preview} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('lists what else the change affects, with the count', () => {
    render(<PreviewDetails preview={{ ...preview, affected: ['widget/blue', 'widget/green'] }} />)
    expect(screen.getByRole('heading', { name: 'Affects 2' })).toBeInTheDocument()
    expect(within(screen.getByRole('list', { name: 'Affected' })).getAllByRole('listitem')).toHaveLength(2)
  })

  it('says when the change asks for a passkey, in the caller’s words', () => {
    const { container } = render(<PreviewDetails preview={{ ...preview, step_up: true }} stepUp="Touch your key." />)
    expect(container.querySelector('[data-step-up]')).toHaveTextContent('Touch your key.')
  })
})

describe('ConfirmDialog', () => {
  it('is closed without a preview', () => {
    render(<ConfirmDialog preview={null} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('describes itself by the summary, and Confirm calls onConfirm only', async () => {
    const onConfirm = vi.fn()
    const onCancel = vi.fn()
    render(<ConfirmDialog preview={preview} title="Remove widget" confirmLabel="Remove" onConfirm={onConfirm} onCancel={onCancel} />)
    const dialog = screen.getByRole('dialog', { name: 'Remove widget' })
    expect(dialog).toHaveAccessibleDescription('Remove Blue widget.')
    expect(dialog.querySelector('[data-preview-summary]')).toHaveTextContent('Remove Blue widget.')
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect(onConfirm).toHaveBeenCalledTimes(1)
    expect(onCancel).not.toHaveBeenCalled()
  })

  it('sends nothing on Cancel or Escape', async () => {
    const onConfirm = vi.fn()
    const onCancel = vi.fn()
    const user = userEvent.setup()
    render(<ConfirmDialog preview={preview} onConfirm={onConfirm} onCancel={onCancel} />)
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    await user.keyboard('{Escape}')
    expect(onCancel).toHaveBeenCalledTimes(2)
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('disables Confirm while busy and shows the busy label', () => {
    render(<ConfirmDialog preview={preview} confirmLabel="Remove" busy busyLabel="Removing…" onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Removing…' })).toBeDisabled()
  })
})

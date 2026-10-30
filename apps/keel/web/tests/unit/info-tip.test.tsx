import { InfoTip } from '@companion/ui'
import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'

// Spec 063 FR-022 / R4: the shared toggletip opens on hover, focus and tap, and closes on
// Escape, blur and a pointer-down outside. Focus stays on the trigger.

function renderTip(footer?: React.ReactNode) {
  render(
    <div>
      <button type="button">before</button>
      <span>
        Trusted signers
        <InfoTip
          label="trusted signers"
          footer={footer}
          data-infotip="trusted-signers"
          contentData={{ 'data-infotip-content': 'trusted-signers' }}
        >
          Signing keys whose commits skip review.
        </InfoTip>
      </span>
      <button type="button">after</button>
    </div>,
  )
  return screen.getByRole('button', { name: 'About trusted signers' })
}

const note = () => screen.queryByRole('note')

describe('InfoTip (spec 063 FR-022)', () => {
  it('renders a labelled trigger with the slot and data attributes, closed', () => {
    const trigger = renderTip()
    expect(trigger).toHaveAttribute('data-slot', 'info-tip-trigger')
    expect(trigger).toHaveAttribute('data-infotip', 'trusted-signers')
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    expect(trigger).not.toHaveAttribute('aria-describedby')
    expect(note()).toBeNull()
  })

  it('opens on mouse hover and closes when the mouse leaves', async () => {
    const trigger = renderTip()
    fireEvent.pointerEnter(trigger, { pointerType: 'mouse' })
    expect(await screen.findByRole('note')).toHaveTextContent('Signing keys whose commits skip review.')
    fireEvent.pointerLeave(trigger, { pointerType: 'mouse' })
    await act(() => new Promise((r) => setTimeout(r, 200)))
    expect(note()).toBeNull()
  })

  it('ignores touch pointerenter (tap opens it through click instead)', () => {
    const trigger = renderTip()
    fireEvent.pointerEnter(trigger, { pointerType: 'touch' })
    expect(note()).toBeNull()
  })

  it('opens on keyboard focus, keeps focus on the trigger, and closes on blur', async () => {
    const user = userEvent.setup()
    const trigger = renderTip()
    screen.getByRole('button', { name: 'before' }).focus()
    await user.tab()
    expect(trigger).toHaveFocus()
    const content = await screen.findByRole('note')
    expect(trigger).toHaveFocus()
    expect(trigger).toHaveAttribute('aria-describedby', content.id)
    expect(content).toHaveAttribute('data-slot', 'info-tip-content')
    expect(content).toHaveAttribute('data-infotip-content', 'trusted-signers')
    await user.tab()
    expect(screen.getByRole('button', { name: 'after' })).toHaveFocus()
    expect(note()).toBeNull()
    expect(trigger).not.toHaveAttribute('aria-describedby')
  })

  it('opens on click, stays open when the mouse leaves, and toggles closed on a second click', async () => {
    const user = userEvent.setup()
    const trigger = renderTip()
    await user.click(trigger)
    expect(await screen.findByRole('note')).toBeInTheDocument()
    fireEvent.pointerLeave(trigger, { pointerType: 'mouse' })
    await act(() => new Promise((r) => setTimeout(r, 200)))
    expect(note()).not.toBeNull()
    await user.click(trigger)
    expect(note()).toBeNull()
  })

  it('closes on Escape', async () => {
    const user = userEvent.setup()
    const trigger = renderTip()
    await user.click(trigger)
    expect(await screen.findByRole('note')).toBeInTheDocument()
    await user.keyboard('{Escape}')
    expect(note()).toBeNull()
  })

  it('closes on a pointer-down outside', async () => {
    const user = userEvent.setup()
    const trigger = renderTip()
    await user.click(trigger)
    expect(await screen.findByRole('note')).toBeInTheDocument()
    await user.click(document.body)
    expect(note()).toBeNull()
  })

  it('renders the footer under the text', async () => {
    const user = userEvent.setup()
    const trigger = renderTip(<a href="/help/reviews#trusted-signers">Learn more</a>)
    await user.click(trigger)
    const content = await screen.findByRole('note')
    const footer = content.querySelector('[data-slot="info-tip-footer"]')
    expect(footer).not.toBeNull()
    expect(footer).toContainElement(screen.getByRole('link', { name: 'Learn more' }))
  })
})

import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { KitProvider, TermTip, type KitContext } from '../../src/kit'

const KIT: KitContext = {
  guideHref: (r) => `/help/${r.guide}`,
  glossary: {
    widget: { term: 'Widget', text: 'A thing the harness lists.', guide: { guide: 'widgets' } },
    gadget: { term: 'Gadget', text: 'Another thing, with no guide.' },
  },
}

const withKit = (term: string) =>
  render(
    <KitProvider value={KIT}>
      <TermTip term={term} />
    </KitProvider>,
  )

afterEach(() => {
  vi.restoreAllMocks()
})

describe('TermTip', () => {
  it('names its trigger after the term and marks it', () => {
    withKit('widget')
    expect(screen.getByRole('button', { name: 'About widget' })).toHaveAttribute('data-infotip', 'widget')
  })

  it('opens on focus with the glossary text and Learn more, and Escape closes it', async () => {
    const user = userEvent.setup()
    withKit('widget')
    await user.tab()
    const note = await screen.findByRole('note')
    expect(note).toHaveAttribute('data-infotip-content', 'widget')
    expect(note).toHaveTextContent('A thing the harness lists.')
    expect(screen.getByRole('link', { name: 'Learn more' })).toHaveAttribute('href', '/help/widgets')
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('note')).toBeNull()
  })

  it('has no Learn more when the entry has no guide', async () => {
    const user = userEvent.setup()
    withKit('gadget')
    await user.tab()
    expect(await screen.findByRole('note')).toHaveTextContent('Another thing, with no guide.')
    expect(screen.queryByRole('link')).toBeNull()
  })

  it('renders nothing, and warns, for a term the glossary lacks', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    const { container } = withKit('sprocket')
    expect(container).toBeEmptyDOMElement()
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('sprocket'))
  })
})

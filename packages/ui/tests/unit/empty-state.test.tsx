import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { EmptyState, emptyVariant, KitProvider } from '../../src/kit'

describe('emptyVariant', () => {
  it('lets a filter win, then picks guided or restricted', () => {
    expect(emptyVariant({ filtered: true, canAct: true })).toBe('no-match')
    expect(emptyVariant({ filtered: true, canAct: false })).toBe('no-match')
    expect(emptyVariant({ filtered: false, canAct: true })).toBe('guided')
    expect(emptyVariant({ filtered: false, canAct: false })).toBe('restricted')
  })
})

function root(container: HTMLElement): HTMLElement {
  const el = container.querySelector('[data-pane-state="empty"]')
  if (!(el instanceof HTMLElement)) throw new Error('no empty state')
  return el
}

describe('EmptyState', () => {
  it('without a page, has no data-empty', () => {
    const { container } = render(<EmptyState title="No widgets yet" />)
    expect(root(container)).not.toHaveAttribute('data-empty')
    expect(root(container)).not.toHaveAttribute('data-empty-page')
  })

  it('a page with an action is guided, numbers its steps, and never says who can', () => {
    const { container } = render(
      <EmptyState page="widgets" title="No widgets yet" steps={['One.', 'Two.']} action={<button>Add widget</button>} whoCan="An admin" verb="add widgets" />,
    )
    expect(root(container)).toHaveAttribute('data-empty', 'guided')
    expect(root(container)).toHaveAttribute('data-empty-page', 'widgets')
    expect(screen.getAllByRole('listitem')).toHaveLength(2)
    expect(container.querySelector('[data-empty-action]')).not.toBeNull()
    expect(container.querySelector('[data-who-can]')).toBeNull()
  })

  it('a page without an action but with whoCan is restricted', () => {
    const { container } = render(<EmptyState page="widgets" title="No widgets yet" whoCan="An owner or admin" verb="add widgets" />)
    expect(root(container)).toHaveAttribute('data-empty', 'restricted')
    expect(container.querySelector('[data-who-can]')).toHaveTextContent('An owner or admin can add widgets.')
  })

  it('links the guide only when the app builds guide routes, named by the title', () => {
    const { unmount } = render(<EmptyState title="No widgets yet" guide={{ guide: 'widgets' }} />)
    expect(screen.queryByRole('link')).toBeNull()
    unmount()
    render(
      <KitProvider value={{ guideHref: (r) => `/help/${r.guide}` }}>
        <EmptyState title="No widgets yet" guide={{ guide: 'widgets' }} />
      </KitProvider>,
    )
    expect(screen.getByRole('link', { name: 'Learn more: No widgets yet' })).toHaveAttribute('href', '/help/widgets')
  })
})

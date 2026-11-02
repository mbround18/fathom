import { CubeIcon, GearIcon, UsersIcon } from '@phosphor-icons/react'
import { render, screen, within } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'

import { NavShell, type NavGroup } from '../../src/kit'

const GROUPS: readonly NavGroup[] = [
  {
    id: 'work',
    label: 'Work',
    items: [
      { path: '/widgets', label: 'Widgets', icon: CubeIcon, badge: { count: 2, label: '2 widgets waiting' } },
      { path: '/gadgets', label: 'Gadgets', icon: CubeIcon, badge: { count: 0, label: 'none waiting' } },
    ],
  },
  { id: 'people', label: 'People', items: [{ path: '/members', label: 'Members', icon: UsersIcon }] },
]

// jsdom has no scrollIntoView; the dot asks for it to stay in view in the phone's strip.
beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn()
})

function shell(path: string) {
  return render(
    <NavShell
      brand={<span>Widget desk</span>}
      groups={GROUPS}
      path={path}
      pinned={{ path: '/settings', label: 'Settings', icon: GearIcon, dot: { label: 'update available' } }}
      headerEnd={<span>Connected</span>}
      banner={<p>Reconnecting</p>}
      overlay={<div data-overlay />}
    >
      <p>The page</p>
    </NavShell>,
  )
}

describe('NavShell', () => {
  it('draws the frame: header, banner, sections, the page and the overlay', () => {
    const { container } = shell('/widgets')
    expect(screen.getByText('Widget desk')).toBeInTheDocument()
    expect(screen.getByText('Connected')).toBeInTheDocument()
    expect(screen.getByText('Reconnecting')).toBeInTheDocument()
    expect(within(screen.getByRole('main')).getByText('The page')).toBeInTheDocument()
    expect(container.querySelector('[data-overlay]')).not.toBeNull()
  })

  it('groups the links under their headings, with data-nav-group', () => {
    shell('/widgets')
    const nav = screen.getByRole('navigation', { name: 'Sections' })
    const work = within(nav).getByRole('group', { name: 'Work' })
    expect(work).toHaveAttribute('data-nav-group', 'work')
    expect(within(work).getAllByRole('link')).toHaveLength(2)
    expect(within(nav).getByRole('group', { name: 'People' })).toHaveAttribute('data-nav-group', 'people')
  })

  it('marks the current section, including below it', () => {
    shell('/widgets/blue')
    expect(screen.getByRole('link', { name: /^Widgets/ })).toHaveAttribute('aria-current', 'page')
    expect(screen.getByRole('link', { name: 'Gadgets' })).not.toHaveAttribute('aria-current')
  })

  it('shows a badge above 0 and reads its label out; hides it at 0', () => {
    const { container } = shell('/members')
    expect(container.querySelector('[data-nav-badge="widgets"]')).toHaveTextContent('2')
    expect(container.querySelector('[data-nav-badge="gadgets"]')).toBeNull()
    expect(screen.getByRole('link', { name: /^Widgets\s*\(2 widgets waiting\)$/ })).toHaveAttribute('href', '/widgets')
  })

  it('pins the extra item last, with its dot read out', () => {
    shell('/settings')
    const links = within(screen.getByRole('navigation')).getAllByRole('link')
    const settings = links.at(-1)
    expect(settings).toHaveAccessibleName(/^Settings\s*\(update available\)$/)
    expect(settings?.querySelector('[data-update-dot]')).not.toBeNull()
    expect(settings).toHaveAttribute('aria-current', 'page')
  })
})

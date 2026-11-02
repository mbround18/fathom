import { ClockCounterClockwiseIcon, CubeIcon, GearIcon, PuzzlePieceIcon, UsersIcon } from '@phosphor-icons/react'

import { NavShell, Page, PageHeader, usePath, type NavGroup, type NavItem } from '../../src/kit'
import type { Fixture } from '../fixture'

const GROUPS: readonly NavGroup[] = [
  {
    id: 'work',
    label: 'Work',
    items: [
      { path: '/widgets', label: 'Widgets', icon: CubeIcon, badge: { count: 3, label: '3 widgets waiting for review' } },
      { path: '/gadgets', label: 'Gadgets', icon: PuzzlePieceIcon, badge: { count: 0, label: 'no gadgets waiting' } },
    ],
  },
  {
    id: 'people',
    label: 'People',
    items: [
      { path: '/members', label: 'Members', icon: UsersIcon },
      { path: '/history', label: 'History', icon: ClockCounterClockwiseIcon },
    ],
  },
]

const SETTINGS: NavItem = { path: '/settings', label: 'Settings', icon: GearIcon, dot: { label: 'update available' } }

const TITLES: Readonly<Record<string, string>> = {
  '/widgets': 'Widgets',
  '/gadgets': 'Gadgets',
  '/members': 'Members',
  '/history': 'History',
  '/settings': 'Settings',
}

/** The shell over the router: the harness starts at `/`, which shows Widgets. */
function Shell({ banner = false }: { banner?: boolean }) {
  const at = usePath()
  const path = at in TITLES ? at : '/widgets'
  return (
    <div className="h-[32rem] border border-border">
      <NavShell
        brand={<span className="px-1 text-sm font-semibold">Widget desk</span>}
        groups={GROUPS}
        path={path}
        pinned={SETTINGS}
        headerEnd={<span className="text-xs text-muted-foreground">Connected</span>}
        banner={
          banner ? (
            <p role="status" className="border-b border-border px-3 py-1.5 text-xs text-muted-foreground">
              Reconnecting… changes are saved when the connection is back.
            </p>
          ) : undefined
        }
      >
        <Page name={(TITLES[path] ?? '').toLowerCase()} labelledBy="shell-title">
          <PageHeader id="shell-title" title={TITLES[path] ?? ''} />
        </Page>
      </NavShell>
    </div>
  )
}

const fixture: Fixture = {
  id: 'nav-shell',
  states: {
    default: () => <Shell />,
    banner: () => <Shell banner />,
  },
}

export default fixture

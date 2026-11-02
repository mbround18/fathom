import {
  ArrowsClockwiseIcon,
  ClipboardTextIcon,
  DevicesIcon,
  MoonIcon,
  RowsIcon,
  SignOutIcon,
  StackIcon,
  SunIcon,
  UsersIcon,
} from '@phosphor-icons/react'
import { BrandMark } from '@companion/theme'
import { Button, ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@companion/ui'
import { Link, NavShell, usePath, type NavGroup, type NavItem } from '@companion/ui/kit'
import { lazy, Suspense, useEffect, type ReactNode } from 'react'

import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { useUpdatesBadge } from '@/features/updates/badge-store'
import { useTabs } from '@/features/tabs/store'
import { useWide } from '@/features/tabs/useWide'

import { useSession } from './session-store'
import { useTheme } from './theme-store'

const TabDock = lazy(() => import('@/features/tabs/TabDock'))
const NetworkGate = lazy(() => import('@/features/network/NetworkGate'))
const PasskeyGate = lazy(() => import('@/features/passkeys/PasskeyGate'))

const CONNECTION_LABEL = {
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  closed: 'Disconnected',
} as const

const STACKS = '/'

/** Stacks with a newer image (spec 062): a count on the Updates item, hidden at zero. */
function updatesItem(count: number | null): NavItem {
  const n = count ?? 0
  return {
    path: '/admin/updates',
    label: 'Updates',
    icon: ArrowsClockwiseIcon,
    badge: { count: n, label: `${String(n)} ${n === 1 ? 'stack has' : 'stacks have'} an update` },
  }
}

/**
 * The sections this person sees (spec 065 T020): stacks and their own sessions, and for an admin
 * the admin pages, with Updates only while the server has that feature on.
 */
function useNav(): NavGroup[] {
  const isAdmin = useSession((s) => s.me?.role === 'admin')
  const updates = useSession((s) => s.me?.features.updates === true)
  const count = useUpdatesBadge((s) => s.count)
  const groups: NavGroup[] = [
    { id: 'stacks', label: 'Stacks', items: [{ path: STACKS, label: 'Stacks', icon: StackIcon }] },
    {
      id: 'account',
      label: 'Account',
      items: [{ path: '/account/sessions', label: 'Your sessions', icon: DevicesIcon }],
    },
  ]
  if (isAdmin)
    groups.push({
      id: 'admin',
      label: 'Admin',
      items: [
        { path: '/admin/users', label: 'Users', icon: UsersIcon },
        { path: '/admin/audit', label: 'Audit', icon: ClipboardTextIcon },
        ...(updates ? [updatesItem(count)] : []),
      ],
    })
  return groups
}

/** A stack's pages (its detail, files and editor) and the new-stack page all sit under Stacks. */
function navPath(path: string): string {
  return path.startsWith('/stacks/') || path === '/new' ? STACKS : path
}

function Dock() {
  return (
    <div data-testid="tab-dock" className="h-full min-h-0 border-t border-border">
      <Suspense fallback={null}>
        <TabDock />
      </Suspense>
    </div>
  )
}

/**
 * The signed-in frame: the kit's `NavShell` (a dense top bar, the grouped section navigation and the
 * routed page) with Depths' sections, the tab toggle, the theme and sign-out. Once a tab is open,
 * the tab dock (logs, shells, stats) sits under the page. Wide screens split the page and the dock
 * with a draggable divider; narrow ones show one or the other, switched from the top bar.
 */
export function Shell({ children }: { children: ReactNode }) {
  const me = useSession((s) => s.me)
  const connection = useSession((s) => s.connection)
  const signOut = useSession((s) => s.signOut)
  const mode = useTheme((s) => s.theme.mode)
  const toggleMode = useTheme((s) => s.toggleMode)
  const count = useTabs((s) => s.layout.tabs.length)
  const open = useTabs((s) => s.layout.open)
  const dockSize = useTabs((s) => s.layout.sizes.dock)
  const setOpen = useTabs((s) => s.setOpen)
  const setSizes = useTabs((s) => s.setSizes)
  const wide = useWide()
  const path = usePath()
  const groups = useNav()

  // Streams run from the first tab on, whether or not the dock is showing.
  useEffect(() => {
    if (count > 0) void import('@/features/tabs/streams')
  }, [count])

  const showDock = count > 0 && open
  const gate = me !== null && (!me.network.trusted || me.needs_answer || me.proxy_warning)
  const page = <div className="h-full min-h-0 overflow-auto">{children}</div>

  return (
    <NavShell
      brand={
        <Link to={STACKS} aria-label="Stacks" className="flex items-center">
          <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-sm" />
        </Link>
      }
      groups={groups}
      path={navPath(path)}
      headerEnd={
        <>
          {connection !== 'open' && (
            <span role="status" data-connection={connection} className="text-xs text-status-warn">
              {CONNECTION_LABEL[connection]}…
            </span>
          )}
          {count > 0 && (
            <Button
              variant="ghost"
              size="sm"
              aria-pressed={open}
              aria-label={open ? `Hide tabs (${String(count)})` : `Show tabs (${String(count)})`}
              data-testid="tabs-toggle"
              onClick={() => setOpen(!open)}
            >
              <RowsIcon aria-hidden />
              <span className="tabular-nums">{count}</span>
            </Button>
          )}
          {me && (
            <span data-testid="whoami" className="hidden max-w-40 truncate text-xs text-muted-foreground sm:inline">
              {me.identity.username}
            </span>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={mode === 'dark' ? 'Use light theme' : 'Use dark theme'}
            onClick={toggleMode}
          >
            {mode === 'dark' ? <SunIcon /> : <MoonIcon />}
          </Button>
          <Button variant="ghost" size="icon-sm" aria-label="Sign out" onClick={() => void signOut()}>
            <SignOutIcon />
          </Button>
        </>
      }
      banner={
        <>
          {gate && (
            <Suspense fallback={null}>
              <NetworkGate />
            </Suspense>
          )}
          {me && (
            <Suspense fallback={null}>
              <PasskeyGate />
            </Suspense>
          )}
        </>
      }
    >
      {wide ? (
        // One group throughout, so opening the dock never remounts the page.
        <ResizablePanelGroup
          orientation="vertical"
          onLayoutChanged={(l, meta) => {
            const dock = l.dock
            if (meta.isUserInteraction && dock !== undefined) setSizes({ dock })
          }}
        >
          <ResizablePanel id="page" minSize="15">
            {page}
          </ResizablePanel>
          {showDock && (
            <>
              <ResizableHandle withHandle aria-label="Resize tabs" />
              <ResizablePanel id="dock" defaultSize={String(dockSize ?? 45)} minSize="15">
                <Dock />
              </ResizablePanel>
            </>
          )}
        </ResizablePanelGroup>
      ) : (
        <div className="h-full min-h-0">
          <div className={showDock ? 'hidden' : 'h-full min-h-0'}>{page}</div>
          {showDock && <Dock />}
        </div>
      )}
    </NavShell>
  )
}

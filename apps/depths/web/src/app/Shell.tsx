import { ClipboardTextIcon, DevicesIcon, MoonIcon, RowsIcon, SignOutIcon, SunIcon, UsersIcon } from '@phosphor-icons/react'
import { BrandMark } from '@companion/theme'
import { Button, ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@companion/ui'
import { lazy, Suspense, useEffect, type ReactNode } from 'react'

import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { useTabs } from '@/features/tabs/store'
import { useWide } from '@/features/tabs/useWide'

import { Link } from './Link'
import { usePath } from './router'
import { useSession } from './session-store'
import { useTheme } from './theme-store'

const TabDock = lazy(() => import('@/features/tabs/TabDock'))
const NetworkGate = lazy(() => import('@/features/network/NetworkGate'))

const CONNECTION_LABEL = {
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  closed: 'Disconnected',
} as const

function AdminLink({ to, label, icon }: { to: string; label: string; icon: ReactNode }) {
  const current = usePath().startsWith(to)
  return (
    <Link
      to={to}
      aria-label={label}
      aria-current={current ? 'page' : undefined}
      className="inline-flex h-7 items-center gap-1 px-1.5 text-xs text-muted-foreground hover:text-foreground aria-[current=page]:text-primary"
    >
      {icon}
      <span className="hidden sm:inline">{label}</span>
    </Link>
  )
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
 * The signed-in frame: a dense top bar, then the routed page and, once a tab is open, the tab dock
 * (logs, shells, stats). Wide screens split the page and the dock with a draggable divider; narrow
 * ones show one or the other, switched from the top bar. Mobile first.
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

  // Streams run from the first tab on, whether or not the dock is showing.
  useEffect(() => {
    if (count > 0) void import('@/features/tabs/streams')
  }, [count])

  const showDock = count > 0 && open
  const gate = me !== null && (!me.network.trusted || me.needs_answer || me.proxy_warning)
  const page = <div className="h-full min-h-0 overflow-auto">{children}</div>

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-2">
        <Link to="/" aria-label="Stacks" className="flex items-center">
          <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-sm" />
        </Link>
        {me?.role === 'admin' && (
          <nav aria-label="Admin" className="flex items-center gap-0.5">
            <AdminLink to="/admin/users" label="Users" icon={<UsersIcon aria-hidden className="size-3.5" />} />
            <AdminLink to="/admin/audit" label="Audit" icon={<ClipboardTextIcon aria-hidden className="size-3.5" />} />
          </nav>
        )}
        <span className="flex-1" />
        {connection !== 'open' && (
          <span
            role="status"
            data-connection={connection}
            className="text-xs text-status-warn"
          >
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
          <Link
            to="/account/sessions"
            aria-label="Your sessions"
            title="Your sessions"
            className="inline-flex h-7 min-w-0 items-center gap-1 px-1.5 text-xs text-muted-foreground hover:text-foreground"
          >
            <DevicesIcon aria-hidden className="size-3.5 shrink-0" />
            <span data-testid="whoami" className="hidden max-w-40 truncate sm:inline">
              {me.identity.username}
            </span>
          </Link>
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
      </header>
      {gate && (
        <Suspense fallback={null}>
          <NetworkGate />
        </Suspense>
      )}
      <main className="min-h-0 flex-1">
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
          <>
            <div className={showDock ? 'hidden' : 'h-full min-h-0'}>{page}</div>
            {showDock && <Dock />}
          </>
        )}
      </main>
    </div>
  )
}

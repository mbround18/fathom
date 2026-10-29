import {
  CaretDownIcon,
  CubeIcon,
  FingerprintIcon,
  KeyIcon,
  ListIcon,
  SignOutIcon,
  UserCircleIcon,
} from '@phosphor-icons/react'
import { BrandMark } from '@companion/theme'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@companion/ui'
import { lazy, Suspense, type ReactNode } from 'react'

import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { usePasskeys } from '@/features/passkeys/store'
import { canSeeSettings } from '@/features/settings/tabs'
import { showDot, useUpdates } from '@/features/settings/updates'

import { Link } from './Link'
import { ACCOUNT_PASSKEYS, HOME, SECTIONS, SETTINGS, type Section } from './nav'
import { navigate, usePath } from './router'
import { useSession } from './session-store'

const PasskeyDialogs = lazy(() => import('@/features/passkeys/PasskeyDialogs'))

const CONNECTION_LABEL = {
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  closed: 'Disconnected',
} as const

const NAV_LINK =
  'relative inline-flex h-7 shrink-0 items-center gap-1.5 border-b border-transparent px-2 text-xs text-muted-foreground ' +
  'hover:text-foreground focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none ' +
  'aria-[current=page]:border-accent-neon aria-[current=page]:text-primary md:border-b-0 md:border-l md:aria-[current=page]:bg-accent-neon/5'

function NavLink({
  section: { path: to, label, icon: Icon },
  path,
  className = '',
  dot = false,
}: {
  section: Section
  path: string
  className?: string
  /** A newer Keel is available (spec 061 FR-012); owners only, see `showDot`. */
  dot?: boolean
}) {
  const current = path === to || path.startsWith(`${to}/`)
  return (
    <Link to={to} aria-current={current ? 'page' : undefined} className={`${NAV_LINK} ${className}`}>
      <Icon aria-hidden className="size-3.5 shrink-0" />
      {label}
      {/* The link is `relative` so the sr-only text stays inside the phone's scrolling strip
          instead of widening the page from wherever the strip scrolled it to. */}
      {dot && (
        <>
          <span aria-hidden data-update-dot className="size-1.5 shrink-0 rounded-full bg-accent-neon shadow-[var(--glow)]" />
          <span className="sr-only"> (update available)</span>
        </>
      )}
    </Link>
  )
}

function Nav() {
  const path = usePath()
  const role = useSession((s) => s.me?.instance_role)
  const me = useSession((s) => s.me)
  const dot = useUpdates((s) => showDot(me, s.status))
  return (
    <nav
      aria-label="Sections"
      className="flex shrink-0 gap-0.5 overflow-x-auto border-b border-border px-1 py-1 md:w-44 md:flex-col md:overflow-x-visible md:border-r md:border-b-0 md:py-2"
    >
      {SECTIONS.map((section) => (
        <NavLink key={section.path} section={section} path={path} />
      ))}
      {/* Pinned to the bottom of the column on desktop; last in the strip on a phone. */}
      {canSeeSettings(role) && (
        <NavLink section={SETTINGS} path={path} dot={dot} className="md:mt-auto md:border-t md:border-border" />
      )}
    </nav>
  )
}

function AccountMenu() {
  const me = useSession((s) => s.me)
  const signOut = useSession((s) => s.signOut)
  const openEnrol = usePasskeys((s) => s.openEnrol)
  const openStepUp = usePasskeys((s) => s.openStepUp)
  if (!me) return null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm" aria-label="Account" className="min-w-0">
          <UserCircleIcon aria-hidden />
          <span data-testid="whoami" className="hidden max-w-40 truncate sm:inline">
            {me.display_name}
          </span>
          <CaretDownIcon aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-48">
        <DropdownMenuLabel className="truncate text-xs">{me.email ?? me.display_name}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => navigate(ACCOUNT_PASSKEYS)}>
          <ListIcon aria-hidden className="size-3.5" />
          Passkeys
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openEnrol(false)}>
          <KeyIcon aria-hidden className="size-3.5" />
          Register a passkey
        </DropdownMenuItem>
        {me.verification.enrolled && (
          <DropdownMenuItem onSelect={openStepUp}>
            <FingerprintIcon aria-hidden className="size-3.5" />
            Verify it’s you
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-[11px] font-normal tracking-wider text-muted-foreground uppercase">About</DropdownMenuLabel>
        <div data-keel-version className="flex items-center gap-1.5 px-2 pb-1.5 text-xs">
          <CubeIcon aria-hidden className="size-3.5 text-muted-foreground" />
          Keel {me.keel_version ?? 'version unknown'}
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => void signOut()}>
          <SignOutIcon aria-hidden className="size-3.5" />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** Shown while the person must register a passkey and has none. */
function EnrolBanner() {
  const need = useSession((s) => s.me !== null && s.me.verification.required && !s.me.verification.enrolled)
  const openEnrol = usePasskeys((s) => s.openEnrol)
  if (!need) return null
  return (
    <div
      role="status"
      data-notice="enrol"
      className="flex items-center gap-2 border-b border-status-warn/40 px-3 py-1.5 text-xs text-status-warn"
    >
      <span className="flex-1">Register a passkey before you approve or run risky changes.</span>
      <Button size="sm" variant="outline" onClick={() => openEnrol(true)}>
        Register
      </Button>
    </div>
  )
}

/**
 * The signed-in frame: a dense top bar, the section navigation (a sidebar on wide screens, a
 * scrolling strip on narrow ones) and the routed page. Mobile first.
 */
export function Shell({ children }: { children: ReactNode }) {
  const connection = useSession((s) => s.connection)
  const dialog = usePasskeys((s) => s.dialog !== null)

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-2">
        <Link to={HOME} aria-label="Home" className="flex items-center">
          <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-sm" />
        </Link>
        <span className="flex-1" />
        {connection !== 'open' && (
          <span role="status" data-connection={connection} className="text-xs text-status-warn">
            {CONNECTION_LABEL[connection]}…
          </span>
        )}
        <AccountMenu />
      </header>
      <EnrolBanner />
      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <Nav />
        <main className="min-h-0 flex-1 overflow-auto">{children}</main>
      </div>
      {dialog && (
        <Suspense fallback={null}>
          <PasskeyDialogs />
        </Suspense>
      )}
    </div>
  )
}

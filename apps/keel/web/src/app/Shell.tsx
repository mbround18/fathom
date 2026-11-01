import {
  CaretDownIcon,
  CubeIcon,
  FingerprintIcon,
  KeyIcon,
  ListIcon,
  QuestionIcon,
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
import { lazy, Suspense, useEffect, useRef, type ReactNode } from 'react'

import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { approvesSomewhere } from '@/features/hosts/workspaces'
import { usePasskeys } from '@/features/passkeys/store'
import { useApprovals } from '@/features/runs/approvals-store'
import { canSeeSettings } from '@/features/settings/tabs'
import { showDot, useUpdates } from '@/features/settings/updates'
import { LABEL } from '@/ui/form'

import { Link } from './Link'
import { ACCOUNT_PASSKEYS, HELP, HOME, SETTINGS, visibleGroups, type NavGroup, type Section } from './nav'
import { navigate, usePath } from './router'
import { useSession } from './session-store'

const PasskeyDialogs = lazy(() => import('@/features/passkeys/PasskeyDialogs'))

const CONNECTION_LABEL = {
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  closed: 'Disconnected',
} as const

const NAV_LINK =
  'relative inline-flex h-8 shrink-0 items-center gap-2 border-b border-transparent px-3 text-sm text-muted-foreground ' +
  'hover:text-foreground focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none ' +
  'aria-[current=page]:border-accent-neon aria-[current=page]:text-primary md:border-b-0 md:border-l md:aria-[current=page]:bg-accent-neon/5'

function NavLink({
  section: { path: to, label, icon: Icon },
  path,
  className = '',
  dot = false,
  badge = 0,
}: {
  section: Section
  path: string
  className?: string
  /** A newer Keel is available (spec 061 FR-012); owners only, see `showDot`. */
  dot?: boolean
  /** A count beside the label, hidden at 0. Neutral, never the accent: it is not where you are. */
  badge?: number
}) {
  const current = path === to || path.startsWith(`${to}/`)
  const dotRef = useRef<HTMLSpanElement>(null)
  // On a phone the strip scrolls and Settings is last, so bring the dot into view when it appears.
  useEffect(() => {
    if (dot) dotRef.current?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [dot])
  return (
    <Link to={to} aria-current={current ? 'page' : undefined} className={`${NAV_LINK} ${className}`}>
      <Icon aria-hidden className="size-4 shrink-0" />
      {label}
      {/* The link is `relative` so the sr-only text stays inside the phone's scrolling strip
          instead of widening the page from wherever the strip scrolled it to. */}
      {badge > 0 && (
        <>
          <span
            aria-hidden
            data-nav-badge={to.slice(1)}
            className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center bg-muted px-1 text-[0.6875rem] leading-none font-medium text-foreground tabular-nums"
          >
            {badge}
          </span>
          <span className="sr-only">
            {' '}
            ({badge} {badge === 1 ? 'run' : 'runs'} waiting for your approval)
          </span>
        </>
      )}
      {dot && (
        <>
          <span ref={dotRef} aria-hidden data-update-dot className="size-1.5 shrink-0 rounded-full bg-accent-neon shadow-[var(--glow)]" />
          <span className="sr-only"> (update available)</span>
        </>
      )}
    </Link>
  )
}

/**
 * One group of sections (spec 063 FR-001 to FR-004). On desktop its heading is a muted label, never
 * the accent, with room above every group after the first. On a phone the heading is only read out
 * and the group runs on in the strip, set off from the one before by a 1px rule.
 */
function NavGroupLinks({ group, first, path }: { group: NavGroup; first: boolean; path: string }) {
  const headingId = `nav-group-${group.id}`
  const me = useSession((s) => s.me)
  const waiting = useApprovals((s) => (approvesSomewhere(me) ? s.total : 0))
  return (
    <div
      role="group"
      aria-labelledby={headingId}
      data-nav-group={group.id}
      className={`relative flex shrink-0 gap-0.5 md:flex-col ${first ? '' : 'max-md:border-l max-md:border-border max-md:pl-0.5 md:mt-3'}`}
    >
      {/* The transparent border lines the heading up with the links' text. The group is `relative` so
          the sr-only heading stays inside the phone's scrolling strip, like the dot's text. */}
      <span id={headingId} className={`${LABEL} sr-only md:not-sr-only md:border-l md:border-transparent md:px-3 md:pb-1`}>
        {group.label}
      </span>
      {group.sections.map((section) => (
        <NavLink key={section.path} section={section} path={path} badge={section.badge === 'approvals' ? waiting : 0} />
      ))}
    </div>
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
      className="flex shrink-0 gap-0.5 overflow-x-auto border-b border-border px-1 py-1 md:w-48 md:flex-col md:overflow-x-visible md:border-r md:border-b-0 md:py-2"
    >
      {visibleGroups(me).map((group, i) => (
        <NavGroupLinks key={group.id} group={group} first={i === 0} path={path} />
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
        {/* The Keel guides, served in the app (spec 063). */}
        <DropdownMenuItem onSelect={() => navigate(HELP)}>
          <QuestionIcon aria-hidden className="size-3.5" />
          Help
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
        <DropdownMenuLabel className="text-xs font-medium text-muted-foreground">About</DropdownMenuLabel>
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
 * The signed-in frame: a dense top bar, the grouped section navigation (a sidebar on wide screens,
 * a scrolling strip on narrow ones) and the routed page. Mobile first.
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

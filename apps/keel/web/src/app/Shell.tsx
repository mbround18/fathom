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
import { Link, navigate, NavShell, usePath, type NavGroup, type NavItem } from '@companion/ui/kit'
import { lazy, Suspense, type ReactNode } from 'react'

import { BRAND_GLYPH, BRAND_NAME } from '@/brand'
import { approvesSomewhere } from '@/features/hosts/workspaces'
import { usePasskeys } from '@/features/passkeys/store'
import { useApprovals } from '@/features/runs/approvals-store'
import { showsSettings } from '@/features/settings/tabs'
import { showDot, useUpdates } from '@/features/settings/updates'
import { ChangeDialog } from '@/features/teams/ChangeDialog'

import { ACCOUNT_PASSKEYS, HELP, HOME, SETTINGS, visibleGroups, type Section } from './nav'
import { useSession } from './session-store'
import { useCurrentWorkspace } from './workspace-store'
import { WorkspaceSwitcher } from './WorkspaceSwitcher'

const PasskeyDialogs = lazy(() => import('@/features/passkeys/PasskeyDialogs'))

const CONNECTION_LABEL = {
  connecting: 'Connecting',
  reconnecting: 'Reconnecting',
  closed: 'Disconnected',
} as const

/** A section as the kit's nav item, with its approvals count (spec 064 US4) when it shows one. */
function navItem({ path, label, icon, badge }: Section, waiting: number): NavItem {
  if (badge !== 'approvals') return { path, label, icon }
  return {
    path,
    label,
    icon,
    badge: { count: waiting, label: `${waiting} ${waiting === 1 ? 'run' : 'runs'} waiting for your approval` },
  }
}

/**
 * The groups this person sees (spec 063 FR-001 to FR-004) and Settings, pinned last, with the
 * update dot when a newer Keel is available (spec 061 FR-012; owners only, see `showDot`).
 */
function useNav(): { groups: NavGroup[]; pinned: NavItem | undefined } {
  const me = useSession((s) => s.me)
  const role = useSession((s) => s.me?.instance_role)
  const workspace = useCurrentWorkspace()?.id ?? null
  const waiting = useApprovals((s) => (approvesSomewhere(me) ? s.total : 0))
  const dot = useUpdates((s) => showDot(me, s.status))
  const groups = visibleGroups(me).map((g) => ({
    id: g.id,
    label: g.label,
    items: g.sections.map((s) => navItem(s, waiting)),
  }))
  const settings = navItem(SETTINGS, waiting)
  const pinned = showsSettings(role, me, workspace) ? (dot ? { ...settings, dot: { label: 'update available' } } : settings) : undefined
  return { groups, pinned }
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
 * The signed-in frame: the kit's `NavShell` (a dense top bar, the grouped section navigation and the
 * routed page) with Keel's sections, account menu, connection state and passkey prompts.
 */
export function Shell({ children }: { children: ReactNode }) {
  const connection = useSession((s) => s.connection)
  const dialog = usePasskeys((s) => s.dialog !== null)
  const path = usePath()
  const { groups, pinned } = useNav()

  return (
    <NavShell
      brand={
        <Link to={HOME} aria-label="Home" className="flex items-center">
          <BrandMark name={BRAND_NAME} glyph={BRAND_GLYPH} className="text-sm" />
        </Link>
      }
      groups={groups}
      path={path}
      pinned={pinned}
      headerEnd={
        <>
          {connection !== 'open' && (
            <span role="status" data-connection={connection} className="text-xs text-status-warn">
              {CONNECTION_LABEL[connection]}…
            </span>
          )}
          <WorkspaceSwitcher />
          <AccountMenu />
        </>
      }
      banner={<EnrolBanner />}
      overlay={
        <>
          {/* The one governance confirm dialog: the switcher's create, Teams, Users and Settings share it. */}
          <ChangeDialog />
          {dialog && (
            <Suspense fallback={null}>
              <PasskeyDialogs />
            </Suspense>
          )}
        </>
      }
    >
      {children}
    </NavShell>
  )
}

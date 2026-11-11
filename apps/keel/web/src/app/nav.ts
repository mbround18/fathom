/**
 * The shell's sections, grouped and in navigation order (spec 063 FR-001). Each is one route and
 * one lazily loaded page (routes.tsx); the shell draws the navigation from `visibleGroups`, and
 * everything else reads the flat `SECTIONS`, derived from the groups so the order lives in one place.
 */
import {
  ArrowsLeftRightIcon,
  ClipboardTextIcon,
  GearSixIcon,
  HardDrivesIcon,
  ListChecksIcon,
  PlayIcon,
  SealCheckIcon,
  TreeStructureIcon,
  UserListIcon,
  UsersThreeIcon,
  WebhooksLogoIcon,
  type Icon,
} from '@phosphor-icons/react'

import type { Me } from '@/protocol/wire'

export interface Section {
  path: string
  label: string
  icon: Icon
  /** Whether this person sees the section; absent means everyone does (FR-004). */
  visible?: (me: Me) => boolean
  /** A count chip beside the label, by what it counts (spec 064 US4). */
  badge?: 'approvals'
}

export interface NavGroup {
  /** Also the group's `data-nav-group`. */
  id: 'pipelines' | 'infrastructure' | 'administration'
  /** The group's heading, in sentence case (FR-002). */
  label: string
  sections: readonly Section[]
}

export const NAV_GROUPS: readonly NavGroup[] = [
  {
    id: 'pipelines',
    label: 'Pipelines',
    sections: [
      { path: '/runs', label: 'Runs', icon: PlayIcon, badge: 'approvals' },
      { path: '/reviews', label: 'Reviews', icon: SealCheckIcon },
      { path: '/bindings', label: 'Bindings', icon: TreeStructureIcon },
      { path: '/receivers', label: 'Receivers', icon: WebhooksLogoIcon },
    ],
  },
  {
    id: 'infrastructure',
    label: 'Infrastructure',
    sections: [
      { path: '/inventory', label: 'Inventory', icon: ListChecksIcon },
      { path: '/hosts', label: 'Hosts', icon: HardDrivesIcon },
    ],
  },
  {
    id: 'administration',
    label: 'Administration',
    sections: [
      { path: '/teams', label: 'Teams', icon: UsersThreeIcon },
      { path: '/users', label: 'Users', icon: UserListIcon },
      { path: '/audit', label: 'Audit', icon: ClipboardTextIcon },
      { path: '/migration', label: 'Migration', icon: ArrowsLeftRightIcon },
    ],
  },
]

export const SECTIONS: readonly Section[] = NAV_GROUPS.flatMap((g) => g.sections)

/**
 * The groups as this person sees them: each keeps the sections it may show, and a group left with
 * none is dropped with its heading and divider (FR-004). Signed out, a gated section is hidden.
 * `groups` is for tests: no section is gated yet.
 */
export function visibleGroups(me: Me | null, groups: readonly NavGroup[] = NAV_GROUPS): NavGroup[] {
  return groups.map((g) => ({
    ...g,
    sections: g.sections.filter((s) => s.visible === undefined || (me !== null && s.visible(me))),
  })).filter((g) => g.sections.length > 0)
}

/**
 * Settings: pinned to the bottom of the nav, for owners, admins and a `ws_admin` of the current
 * workspace (`showsSettings`, spec 066 R10). `/settings` opens its first tab, Workspace. For owners it carries a dot while a newer
 * Keel is available and they haven't hidden it for that digest (spec 061 FR-012, FR-014; the rule
 * is `showDot` in features/settings/updates.ts, hidden from the update card).
 */
export const SETTINGS: Section = { path: '/settings', label: 'Settings', icon: GearSixIcon }

/** Account → Passkeys: the person's own passkeys. Reached from the account menu, not the sections. */
export const ACCOUNT_PASSKEYS = '/account/passkeys'

/** Account → Help: the Keel guides (spec 063, help/HelpPage.tsx). Reached from the account menu. */
export const HELP = '/help'

/** Where `/` goes. */
export const HOME = '/runs'

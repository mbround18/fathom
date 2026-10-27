/**
 * The shell's sections, in navigation order. Each is one route and one lazily loaded page
 * (routes.tsx); the shell draws the navigation from this list.
 */
import {
  ArrowsLeftRightIcon,
  ClipboardTextIcon,
  HardDrivesIcon,
  ListChecksIcon,
  PlayIcon,
  SealCheckIcon,
  TreeStructureIcon,
  UsersThreeIcon,
  WebhooksLogoIcon,
  type Icon,
} from '@phosphor-icons/react'

export interface Section {
  path: string
  label: string
  icon: Icon
}

export const SECTIONS: readonly Section[] = [
  { path: '/runs', label: 'Runs', icon: PlayIcon },
  { path: '/reviews', label: 'Reviews', icon: SealCheckIcon },
  { path: '/bindings', label: 'Bindings', icon: TreeStructureIcon },
  { path: '/receivers', label: 'Receivers', icon: WebhooksLogoIcon },
  { path: '/inventory', label: 'Inventory', icon: ListChecksIcon },
  { path: '/hosts', label: 'Hosts', icon: HardDrivesIcon },
  { path: '/teams', label: 'Teams', icon: UsersThreeIcon },
  { path: '/audit', label: 'Audit', icon: ClipboardTextIcon },
  { path: '/migration', label: 'Migration', icon: ArrowsLeftRightIcon },
]

/** Where `/` goes. */
export const HOME = '/runs'

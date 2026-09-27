/** 043 US3: the Settings pages, shared by the header menu and the phone overflow menu. */
import {
  ClipboardTextIcon,
  CubeIcon,
  IdentificationCardIcon,
  type Icon,
  PaletteIcon,
  PlugsIcon,
  SlidersHorizontalIcon,
  UsersThreeIcon,
} from '@phosphor-icons/react'

import { SECTION } from '@/protocol/commands'
import type { SectionKey } from '@/protocol/types'

export interface SettingsItem {
  section: SectionKey
  label: string
  icon: Icon
  /** Local clusters need a local machine: desktop only, never hosted. */
  desktopOnly?: true
}

/** Settings pages in menu order (contracts/ui-shell.md "Settings menu items"). */
export const SETTINGS_ITEMS: readonly SettingsItem[] = [
  { section: SECTION.clusterSettings, label: 'Cluster settings', icon: SlidersHorizontalIcon },
  { section: 'settings/sign-in', label: 'Sign-in', icon: IdentificationCardIcon },
  { section: SECTION.teams, label: 'Team', icon: UsersThreeIcon },
  { section: SECTION.teamProfiles, label: 'Connection profiles', icon: PlugsIcon },
  { section: SECTION.appSettings, label: 'App settings', icon: PaletteIcon },
  { section: SECTION.localClusters, label: 'Local clusters', icon: CubeIcon, desktopOnly: true },
  { section: SECTION.auditLog, label: 'Audit log', icon: ClipboardTextIcon },
]

/** The Settings pages available in this mode. */
export function settingsItems(hosted: boolean): readonly SettingsItem[] {
  return SETTINGS_ITEMS.filter((item) => !(hosted && item.desktopOnly))
}

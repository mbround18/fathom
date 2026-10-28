/**
 * System Settings' tabs and who sees them (spec 060, contracts/web-ui.md "Navigation"): owners and
 * admins see Enrollment, Tokens and Identity; only owners see System. An unknown or forbidden tab
 * goes to Enrollment. The server decides again on every command; this only decides what to draw.
 */
import type { InstanceRole } from '@/protocol/wire'

export type SettingsTab = 'enrollment' | 'tokens' | 'identity' | 'system'

export const DEFAULT_TAB: SettingsTab = 'enrollment'

export const TAB_LABEL: Record<SettingsTab, string> = {
  enrollment: 'Enrollment',
  tokens: 'Tokens',
  identity: 'Identity',
  system: 'System',
}

/** Whether `role` sees System Settings at all (the nav entry and the page). */
export function canSeeSettings(role: InstanceRole | null | undefined): boolean {
  return role === 'owner' || role === 'admin'
}

/** The tabs `role` sees, in order. */
export function visibleTabs(role: InstanceRole | null | undefined): SettingsTab[] {
  if (!canSeeSettings(role)) return []
  const tabs: SettingsTab[] = ['enrollment', 'tokens', 'identity']
  if (role === 'owner') tabs.push('system')
  return tabs
}

/** `tab` when `role` may see it, else the default tab. */
export function resolveTab(tab: string | null | undefined, role: InstanceRole | null | undefined): SettingsTab {
  const visible = visibleTabs(role)
  return visible.find((t) => t === tab) ?? DEFAULT_TAB
}

export function settingsPath(tab: SettingsTab): string {
  return `/settings/${tab}`
}

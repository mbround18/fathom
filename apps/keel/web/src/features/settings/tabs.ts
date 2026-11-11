/**
 * Settings' tabs and who sees them (spec 060, contracts/web-ui.md "Navigation"; spec 066 R10).
 * Owners and admins see Workspace, Enrollment, Tokens, Identity and System. A `ws_admin` of the
 * current workspace sees Workspace alone, and the nav entry with it. The System tab is owner and
 * admin since spec 061 (FR-024), with sections shaped by role. An unknown or forbidden tab goes to
 * Workspace. The server decides again on every command; this only decides what to draw.
 */
import { canAdminister } from '@/features/hosts/workspaces'
import type { InstanceRole, Me } from '@/protocol/wire'

export type SettingsTab = 'workspace' | 'enrollment' | 'tokens' | 'identity' | 'system'

export const DEFAULT_TAB: SettingsTab = 'workspace'

export const TAB_LABEL: Record<SettingsTab, string> = {
  workspace: 'Workspace',
  enrollment: 'Enrollment',
  tokens: 'Tokens',
  identity: 'Identity',
  system: 'System',
}

/** Whether `role` sees the instance-wide settings (and may create a workspace). */
export function canSeeSettings(role: InstanceRole | null | undefined): boolean {
  return role === 'owner' || role === 'admin'
}

/** The tabs this person sees, in order, with `workspace` the current workspace's id. */
export function visibleTabs(
  role: InstanceRole | null | undefined,
  me: Me | null = null,
  workspace: string | null = null,
): SettingsTab[] {
  if (canSeeSettings(role)) return ['workspace', 'enrollment', 'tokens', 'identity', 'system']
  if (workspace !== null && canAdminister(me, workspace)) return ['workspace']
  return []
}

/** Whether the nav shows Settings: whenever any tab is visible. */
export function showsSettings(role: InstanceRole | null | undefined, me: Me | null, workspace: string | null): boolean {
  return visibleTabs(role, me, workspace).length > 0
}

/** `tab` when it is visible, else the default tab. */
export function resolveTab(
  tab: string | null | undefined,
  role: InstanceRole | null | undefined,
  me: Me | null = null,
  workspace: string | null = null,
): SettingsTab {
  return visibleTabs(role, me, workspace).find((t) => t === tab) ?? DEFAULT_TAB
}

export function settingsPath(tab: SettingsTab): string {
  return `/settings/${tab}`
}

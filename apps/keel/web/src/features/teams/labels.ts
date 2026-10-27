/** Display names for the governance roles (spec 055 US6). */
import type { InstanceRole, TeamRole } from '@/protocol/wire'

export const ROLE_LABEL: Record<TeamRole, string> = {
  viewer: 'Viewer',
  operator: 'Operator',
  approver: 'Approver',
  ws_admin: 'Workspace admin',
}

export const TEAM_ROLES: readonly TeamRole[] = ['viewer', 'operator', 'approver', 'ws_admin']

export const INSTANCE_ROLE_LABEL: Record<InstanceRole, string> = {
  owner: 'Owner',
  admin: 'Admin',
  user: 'User',
}

export const INSTANCE_ROLES: readonly InstanceRole[] = ['owner', 'admin', 'user']

/** A profile name as the protocol checks it (`ProfileName`): a slug-like token. */
export const PROFILE_PATTERN = /^[a-z0-9][a-z0-9_-]{0,62}$/

/** At most this many profiles per grant (`GrantProfiles`). */
export const MAX_PROFILES = 32

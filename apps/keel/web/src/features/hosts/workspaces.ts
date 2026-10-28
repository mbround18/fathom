/**
 * The workspaces the caller can see, from `ListWorkspaces` (spec 060 R6), and which of them they
 * can operate on. The server answers every workspace to an instance owner or admin and the
 * granted ones to anyone else, so an owner with no grants still sees them all; `me.grants` alone
 * would show that owner nothing.
 *
 * Operating (enrolling a host) needs an instance owner or admin, or a grant of `operator` or
 * `ws_admin` (keel-server authz). The server decides again on every command; this only decides
 * whether to offer the button.
 */
import { useEffect } from 'react'

import { useSession } from '@/app/session-store'
import { useGovernance, type Refusal } from '@/features/teams/store'
import type { Me, WorkspaceInfo } from '@/protocol/wire'

export function canOperate(me: Me | null, workspace: string): boolean {
  if (!me) return false
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return true
  return me.grants.some((g) => g.workspace === workspace && (g.role === 'operator' || g.role === 'ws_admin'))
}

export interface Workspaces {
  /** `null` while loading. Archived ones are left out. */
  all: WorkspaceInfo[] | null
  operable: WorkspaceInfo[]
  refusal: Refusal | undefined
}

export function useWorkspaces(): Workspaces {
  const me = useSession((s) => s.me)
  const items = useGovernance((s) => s.workspaces)
  const refusal = useGovernance((s) => s.refused.workspaces)
  const load = useGovernance((s) => s.load)
  useEffect(() => {
    load('workspaces')
  }, [load])
  const all = items?.filter((w) => !w.archived) ?? null
  return { all, operable: (all ?? []).filter((w) => canOperate(me, w.id)), refusal }
}

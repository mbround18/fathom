/**
 * The workspaces the caller can see, from `ListWorkspaces` (spec 060 R6), and which of them they
 * can operate on. The server answers every workspace to an instance owner or admin and the
 * granted ones to anyone else, so an owner with no grants still sees them all; `me.grants` alone
 * would show that owner nothing.
 *
 * Operating (enrolling a host, starting a run) needs an instance owner or admin, or a grant of
 * `operator` or `ws_admin`. Administering (receivers, credentials, bindings, inventories, trusted
 * signers, removing a host) needs an owner or admin, or `ws_admin` (keel-core authz
 * `Capability::of`). Approving or denying a held run (spec 064) needs an owner or admin, or an
 * `approver` or `ws_admin` grant on the whole workspace: a grant scoped to profiles covers hosts,
 * not the workspace's runs (keel-core authz `effective`). The server decides again on every
 * command; these only decide whether to offer the button.
 */
import { useCallback, useEffect } from 'react'

import { useSession } from '@/app/session-store'
import { useGovernance, type Refusal } from '@/features/teams/store'
import type { Me, WorkspaceInfo } from '@/protocol/wire'

export function canOperate(me: Me | null, workspace: string): boolean {
  if (!me) return false
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return true
  return me.grants.some((g) => g.workspace === workspace && (g.role === 'operator' || g.role === 'ws_admin'))
}

export function canAdminister(me: Me | null, workspace: string): boolean {
  if (!me) return false
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return true
  return me.grants.some((g) => g.workspace === workspace && g.role === 'ws_admin')
}

export function canApprove(me: Me | null, workspace: string): boolean {
  if (!me) return false
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return true
  return me.grants.some(
    (g) => g.workspace === workspace && g.profiles.length === 0 && (g.role === 'approver' || g.role === 'ws_admin'),
  )
}

/** Whether `me` may approve held runs in any workspace (spec 064 US4: the Runs nav chip). */
export function approvesSomewhere(me: Me | null): boolean {
  if (!me) return false
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return true
  return me.grants.some((g) => canApprove(me, g.workspace))
}

export interface Workspaces {
  /** `null` while loading. Archived ones are left out. */
  all: WorkspaceInfo[] | null
  operable: WorkspaceInfo[]
  refusal: Refusal | undefined
  /** Asks for the workspaces again, after a failed load. */
  retry: () => void
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
  const retry = useCallback(() => {
    load('workspaces')
  }, [load])
  return { all, operable: (all ?? []).filter((w) => canOperate(me, w.id)), refusal, retry }
}

/**
 * 043 US8: a one-shot "open the role form" request from outside the Roles toolbar (the command
 * palette's "New Role…"/"New ClusterRole…" actions). The palette navigates to the Roles or
 * ClusterRoles list; that list's `RoleCreateLauncher` takes the request when it mounts, so the
 * form still opens through 042's launcher and access gating (a denied launcher ignores it).
 */
import { create } from 'zustand'

import type { RoleKind } from './roleYaml'

/** A request older than this is stale (the list never finished loading) and is dropped. */
export const ROLE_CREATE_REQUEST_TTL_MS = 10_000

interface RoleCreateRequest {
  kind: RoleKind
  at: number
}

export const useRoleCreateRequest = create<{ request: RoleCreateRequest | undefined }>(() => ({
  request: undefined,
}))

export function requestRoleCreate(kind: RoleKind): void {
  useRoleCreateRequest.setState({ request: { kind, at: Date.now() } })
}

/** Take the pending request for `kind`, if there is a fresh one. */
export function takeRoleCreateRequest(kind: RoleKind, now = Date.now()): boolean {
  const { request } = useRoleCreateRequest.getState()
  if (!request || request.kind !== kind) return false
  useRoleCreateRequest.setState({ request: undefined })
  return now - request.at <= ROLE_CREATE_REQUEST_TTL_MS
}

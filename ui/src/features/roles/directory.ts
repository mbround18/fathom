/**
 * The subject picker's view of the IdP directory. Kept behind this small interface so the
 * dialog does not care whether the data comes from the store slice fed by `IdpDirectoryUpdated`
 * or, in tests, a fake transport.
 */
import type { IdpGroup, IdpUser } from '@/protocol/types'
import { useStore } from '@/store'

export interface DirectorySource {
  users: IdpUser[]
  groups: IdpGroup[]
  stale: boolean
  error: string | null
}

const EMPTY: DirectorySource = { users: [], groups: [], stale: false, error: null }

export function useStoreDirectory(issuerId: string): DirectorySource {
  const snap = useStore((s) => s.roles.directory[issuerId])
  if (!snap) return EMPTY
  return { users: snap.users, groups: snap.groups, stale: snap.stale, error: snap.error }
}

/** Case-insensitive substring filter, enabled users only. */
export function filterUsers(users: readonly IdpUser[], q: string): IdpUser[] {
  const needle = q.trim().toLowerCase()
  return users.filter((u) => u.enabled && u.username.toLowerCase().includes(needle))
}

export function filterGroups(groups: readonly IdpGroup[], q: string): IdpGroup[] {
  const needle = q.trim().toLowerCase()
  return groups.filter((g) => g.name.toLowerCase().includes(needle))
}

/** Pure helpers for the users view (US5, T074). */
import type { User } from '@/protocol/generated'

/** `<provider>:<subject>`, as the server keys its users list. Never a display name. */
const USER_ID = /^(discord|github|oidc):[^\s:][^\s]*$/

export function isUserId(id: string): boolean {
  return USER_ID.test(id)
}

/** Assigned stacks with no directory on disk (the server keeps them; the UI marks them). */
export function missingStacks(user: Pick<User, 'stacks'>, known: ReadonlySet<string>): string[] {
  return user.stacks.filter((s) => !known.has(s))
}

/** Every stack the editor offers: the ones on disk, then any assigned ones that are missing. */
export function stackChoices(known: readonly string[], assigned: readonly string[]): string[] {
  const set = new Set(known)
  return [...[...known].sort(), ...assigned.filter((s) => !set.has(s)).sort()]
}

/** How the list names a person: their last-seen username, or the id for one never seen. */
export function displayName(u: Pick<User, 'username' | 'id'>): string {
  return u.username ?? u.id
}

/** Owners first, then admins, then by name. */
export function sortUsers(users: readonly User[]): User[] {
  const rank = (u: User): number => (u.owner ? 0 : u.role === 'admin' ? 1 : 2)
  return [...users].sort((a, b) => rank(a) - rank(b) || displayName(a).localeCompare(displayName(b)))
}

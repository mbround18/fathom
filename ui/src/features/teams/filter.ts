/**
 * 033 US3 (R8): advisory visibility of shared profiles by IdP group. A profile with no links is
 * visible to everyone; a linked profile is visible to members of any linked group. This is a UI
 * hint over the self-hosted team store, not access control.
 */
import type { TeamLink } from '@/protocol/types'

export function isProfileVisible(
  profileId: string,
  links: readonly TeamLink[],
  myGroups: readonly string[],
): boolean {
  const linked = links.filter((l) => l.profile_id === profileId)
  if (linked.length === 0) return true
  return linked.some((l) => myGroups.includes(l.group))
}

export function filterProfiles<T extends { id: string }>(
  profiles: readonly T[],
  links: readonly TeamLink[],
  myGroups: readonly string[],
): T[] {
  return profiles.filter((p) => isProfileVisible(p.id, links, myGroups))
}

/**
 * A host's profiles (spec 068 US3, contracts/ui.md "Host page"): what the Profiles card and the
 * profile editor derive from `HostInfo` and the server's answers. Pure, so Vitest covers it.
 */
import type { HostInfo, PlannedStep, ProfileSuggestion } from '@/protocol/wire'

/** Assigned profiles the host's own ceiling (`keel_runner_allowed_profiles`) leaves out. */
export function outsideCeiling(host: Pick<HostInfo, 'profiles' | 'allowed_profiles'>): string[] {
  return host.profiles.filter((p) => !host.allowed_profiles.includes(p))
}

/** What setting `current` to `next` adds and removes, in the order they appear. */
export function changeOf(current: readonly string[], next: readonly string[]): { add: string[]; remove: string[] } {
  return {
    add: next.filter((p) => !current.includes(p)),
    remove: current.filter((p) => !next.includes(p)),
  }
}

/** The profiles after a suggestion is accepted. */
export function applySuggestion(current: readonly string[], s: Pick<ProfileSuggestion, 'profile' | 'action'>): string[] {
  if (s.action === 'add') return current.includes(s.profile) ? [...current] : [...current, s.profile]
  return current.filter((p) => p !== s.profile)
}

function list(items: readonly string[]): string {
  return items.join(', ')
}

/** Why the inventory suggests it: "from k3s_agents", or for a removal, which groups no longer map to it. */
export function suggestionReason(s: Pick<ProfileSuggestion, 'action' | 'groups'>): string {
  if (s.action === 'add') return s.groups.length > 0 ? `from ${list(s.groups)}` : 'from the inventory'
  return s.groups.length > 0 ? `no group maps to it (in ${list(s.groups)})` : 'no group maps to it'
}

/** The ceiling line under the chips. */
export function ceilingText(allowed: readonly string[]): string {
  return allowed.length === 0 ? 'Its runner has not reported which profiles it accepts.' : `This host accepts: ${list(allowed)}.`
}

/** "2 suggested from inventory groups", for the Hosts list's dot. */
export function suggestionCount(n: number): string {
  return `${String(n)} suggested from inventory groups`
}

/** A planned step's line in the profile preview: "debian (from keel-node → debian)". */
export function stepLine(step: PlannedStep): string {
  return step.via.length > 0 ? `${step.job} (from ${step.via.join(' → ')})` : step.job
}

/** Removes the suggestions a profile change settles, as the server does on Confirm. */
export function settled(items: readonly ProfileSuggestion[], after: readonly string[]): ProfileSuggestion[] {
  return items.filter((s) => (s.action === 'add' ? !after.includes(s.profile) : after.includes(s.profile)))
}

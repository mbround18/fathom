/**
 * 033 US3: IdP directory, admin-key presence and advisory team links, fed by `Idp*` /
 * `TeamLinksUpdated` system events. Read-only mirror; the backend owns the data (US4 reads the
 * same `directory` slice for its picker).
 */
import type { IdpDirectorySnapshot, SystemEvent, TeamLink } from '@/protocol/types'

export interface TeamsState {
  /** Last-good directory per issuer id (kept, flagged `stale`, when a refresh fails). */
  directory: Record<string, IdpDirectorySnapshot>
  /** Whether a read-only admin API key is stored, per issuer id. */
  adminKey: Record<string, boolean>
  /** Advisory group -> shared-profile links from the team store. */
  links: TeamLink[]
}

export function initialTeamsState(): TeamsState {
  return { directory: {}, adminKey: {}, links: [] }
}

/** Applies one event; returns false when it is not a Teams event. */
export function reduceTeams(s: TeamsState, event: SystemEvent): boolean {
  switch (event.type) {
    case 'IdpDirectoryUpdated':
      s.directory = { ...s.directory, [event.data.issuer_id]: event.data }
      return true
    case 'IdpAdminKeyStatus':
      s.adminKey = { ...s.adminKey, [event.data.issuer_id]: event.data.present }
      return true
    case 'TeamLinksUpdated':
      s.links = event.data.links
      return true
    default:
      return false
  }
}

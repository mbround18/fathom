/**
 * The fake backend's host profiles and inventory suggestions (spec 068 US3, T040), answering as
 * apps/keel/server does, installed on a `FakeBackend` with `on(...)` and `chain('Confirm', ...)`:
 *
 * - `PreviewHostProfiles` refuses an empty change (`no_change`), a host it does not know
 *   (`not_found`) or a profile outside `known` (`unknown_profile`), else answers
 *   `HostProfilesPreview` (outside the host's ceiling, principals after, `planFor` steps after)
 *   and `Preview` with `step_up`.
 * - Its `Confirm` needs the `step_up` assertion or a verified session (`step_up_required`), sets
 *   the host's profiles in `FakeHosts`, settles suggestions, answers `Confirmed` and `Host`, and
 *   pushes `HostProfilesChanged`.
 * - `ListProfileSuggestions` and `DismissProfileSuggestion` answer `ProfileSuggestions`;
 *   `disabled` answers `feature_disabled` instead (KEEL_CONVERGE off).
 * - `sync(items)` stands in for a repository sync that moved suggestions: they are replaced, the
 *   hosts' counts follow, and `ProfileSuggestionsChanged` is pushed.
 */
import type { Cmd, Event, HostInfo, PlannedStep, ProfileSuggestion } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import type { FakeHosts } from './fake-hosts'
import { NOW } from './fake-repos'

const EXPIRES = '2026-10-27T12:15:00Z'

export function suggestion(over: Partial<ProfileSuggestion> & Pick<ProfileSuggestion, 'host_id' | 'profile'>): ProfileSuggestion {
  return { action: 'add', groups: ['k3s_agents'], first_seen_at: NOW, ...over }
}

interface Pending {
  host: string
  after: string[]
}

export class FakeProfiles {
  /** Profiles the repository knows. */
  known = ['base', 'keel-node', 'k3s-agent', 'web', 'debian']
  suggestions: ProfileSuggestion[] = []
  /** KEEL_CONVERGE off: suggestions answer `feature_disabled`. */
  disabled = false
  /** The converge steps a host would run with `profiles`. */
  planFor: (profiles: readonly string[]) => PlannedStep[] = (profiles) =>
    profiles.map((p, i) => ({ position: i + 1, job: p, via: [p], params: {}, credential_names: [] }))
  /** Every preview asked for, in order. */
  readonly previewed: { host_id: string; add: string[]; remove: string[] }[] = []
  /** Every Dismiss, in order. */
  readonly dismissed: ProfileSuggestion[] = []
  private readonly pending = new Map<string, Pending>()
  private seq = 0
  private readonly backend: FakeBackend
  private readonly hosts: FakeHosts
  private readonly binding: string

  constructor(backend: FakeBackend, hosts: FakeHosts, binding = 'binding-ops') {
    this.backend = backend
    this.hosts = hosts
    this.binding = binding
    for (const type of ['PreviewHostProfiles', 'ListProfileSuggestions', 'DismissProfileSuggestion'] as const) {
      backend.on(type, (cmd) => this.respond(cmd))
    }
    backend.chain('Confirm', (cmd) => this.respond(cmd))
  }

  /** Sets the open suggestions without telling anyone (before the page loads). */
  seed(items: ProfileSuggestion[]): void {
    this.suggestions = items
    this.recount()
  }

  /** A sync moved the suggestions: replaced, counted, and `ProfileSuggestionsChanged` pushed. */
  sync(items: ProfileSuggestion[]): void {
    const touched = new Set([...this.suggestions, ...items].map((s) => s.host_id))
    this.seed(items)
    this.backend.push({ type: 'ProfileSuggestionsChanged', data: { binding_id: this.binding, hosts: [...touched] } })
  }

  private recount(): void {
    this.hosts.hosts = this.hosts.hosts.map((h) => ({
      ...h,
      suggestions: this.suggestions.filter((s) => s.host_id === h.id).length,
    }))
  }

  private of(host: string): ProfileSuggestion[] {
    return this.suggestions.filter((s) => s.host_id === host)
  }

  private off(): Event[] {
    return [{ type: 'OpError', data: { code: 'feature_disabled', message: 'Converge is not enabled on this Keel.' } }]
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'PreviewHostProfiles': {
        const { host_id, add, remove } = cmd.data
        this.previewed.push({ host_id, add, remove })
        const host = this.hosts.hosts.find((h) => h.id === host_id)
        if (!host) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such host.' } }]
        if (add.length === 0 && remove.length === 0) {
          return [{ type: 'OpError', data: { code: 'no_change', message: 'That changes nothing.' } }]
        }
        const unknown = add.find((p) => !this.known.includes(p))
        if (unknown !== undefined) {
          return [{ type: 'OpError', data: { code: 'unknown_profile', message: `No repository defines the profile ${unknown}.` } }]
        }
        const after = [...host.profiles.filter((p) => !remove.includes(p)), ...add.filter((p) => !host.profiles.includes(p))]
        const preview_id = `profiles-preview-${String(++this.seq)}`
        this.pending.set(preview_id, { host: host_id, after })
        return [
          {
            type: 'HostProfilesPreview',
            data: {
              preview_token: preview_id,
              preview: {
                change: { host_id, add, remove },
                outside_ceiling: add.filter((p) => !host.allowed_profiles.includes(p)),
                principals_after: [host.inventory_name, ...after.filter((p) => host.allowed_profiles.includes(p)).map((p) => `profile:${p}`)],
                plan_after: this.planFor(after),
              },
            },
          },
          {
            type: 'Preview',
            data: {
              preview_id,
              action: 'host_profiles_set',
              summary: `Set the profiles of ${host.inventory_name} to ${after.join(', ') || 'none'}.`,
              affected: [host.inventory_name],
              step_up: true,
              expires_at: EXPIRES,
            },
          },
        ]
      }
      case 'ListProfileSuggestions': {
        if (this.disabled) return this.off()
        const { host_id } = cmd.data
        return [{ type: 'ProfileSuggestions', data: { items: host_id ? this.of(host_id) : this.suggestions } }]
      }
      case 'DismissProfileSuggestion': {
        if (this.disabled) return this.off()
        const { host_id, profile, action } = cmd.data
        const hit = this.suggestions.find((s) => s.host_id === host_id && s.profile === profile && s.action === action)
        if (!hit) return [{ type: 'OpError', data: { code: 'not_found', message: 'That suggestion is gone.' } }]
        this.dismissed.push(hit)
        this.seed(this.suggestions.filter((s) => s !== hit))
        return [{ type: 'ProfileSuggestions', data: { items: this.of(host_id) } }]
      }
      case 'Confirm': {
        const { preview_id, step_up } = cmd.data
        const p = this.pending.get(preview_id)
        if (p === undefined) return undefined
        if (!step_up && !this.backend.me?.verification.verified) {
          return [{ type: 'OpError', data: { code: 'step_up_required', message: 'Confirm it’s you with a passkey first.' } }]
        }
        this.pending.delete(preview_id)
        let changed: HostInfo | undefined
        this.hosts.hosts = this.hosts.hosts.map((h) => (h.id === p.host ? (changed = { ...h, profiles: p.after }) : h))
        if (!changed) return [{ type: 'OpError', data: { code: 'host_revoked', message: 'The host is gone.' } }]
        // A change settles the suggestions it answers, as the server does.
        this.seed(
          this.suggestions.filter(
            (s) => s.host_id !== p.host || (s.action === 'add' ? !p.after.includes(s.profile) : p.after.includes(s.profile)),
          ),
        )
        const host = this.hosts.hosts.find((h) => h.id === p.host) ?? changed
        this.backend.push({ type: 'HostProfilesChanged', data: { host_id: host.id, profiles: host.profiles } })
        return [
          { type: 'Confirmed', data: { preview_id } },
          {
            type: 'Host',
            data: { host, python_version: null, override_quorum: null, ledger_head: null, certs: [], outside_runs: [] },
          },
        ]
      }
      default:
        return undefined
    }
  }
}

/**
 * Image updates (spec 062 US1): labels, tones and filters for the Updates page and the stack page.
 * Pure, so the page and the unit tests share it.
 */
import type { Exclusion, Me, RegistryAuth, ServiceUpdate, UpdateSettings, UpdateState } from '@/protocol/generated'

export const STATE_LABEL: Record<UpdateState, string> = {
  not_checked: 'Not checked',
  up_to_date: 'Up to date',
  available: 'Update available',
  moved_back: 'Tag moved back',
  rejected: 'Rejected',
  held: 'Held',
  excluded: 'Excluded',
  updating: 'Updating',
  failed: 'Failed',
  rolled_back: 'Rolled back',
  rollback_failed: 'Rollback failed',
  interrupted: 'Interrupted',
  check_failed: "Can't check",
}

/** The order of the state filter and of the counts: what needs a person first. */
export const STATE_ORDER: readonly UpdateState[] = [
  'available',
  'check_failed',
  'held',
  'moved_back',
  'rejected',
  'failed',
  'rollback_failed',
  'rolled_back',
  'interrupted',
  'updating',
  'excluded',
  'not_checked',
  'up_to_date',
]

/** Border and text tone; the label always carries the meaning (WCAG 1.4.1). */
export const STATE_TONE: Record<UpdateState, string> = {
  not_checked: 'border-border text-muted-foreground',
  up_to_date: 'border-status-ok/60 text-status-ok',
  available: 'border-primary text-primary',
  moved_back: 'border-status-warn/60 text-status-warn',
  rejected: 'border-status-warn/60 text-status-warn',
  held: 'border-status-warn/60 text-status-warn',
  excluded: 'border-dashed border-border text-muted-foreground',
  updating: 'border-primary/60 text-primary',
  failed: 'border-status-err/60 text-status-err',
  rolled_back: 'border-status-warn/60 text-status-warn',
  rollback_failed: 'border-status-err/60 text-status-err',
  interrupted: 'border-status-err/60 text-status-err',
  check_failed: 'border-status-err/60 text-status-err',
}

export type StateFilter = UpdateState | 'all'

/** Services matching the state and stack filters, in the server's order (stack, then service). */
export function filterServices(services: readonly ServiceUpdate[], state: StateFilter, stack: string): ServiceUpdate[] {
  return services.filter((s) => (state === 'all' || s.state === state) && (stack === '' || s.stack === stack))
}

/** How many services are in each state. */
export function countStates(services: readonly ServiceUpdate[]): Partial<Record<UpdateState, number>> {
  const counts: Partial<Record<UpdateState, number>> = {}
  for (const s of services) counts[s.state] = (counts[s.state] ?? 0) + 1
  return counts
}

/** Distinct stack names, sorted. */
export function stacksOf(services: readonly ServiceUpdate[]): string[] {
  return [...new Set(services.map((s) => s.stack))].sort()
}

/** `sha256:0123456789ab…` → `0123456789ab`. */
export function shortDigest(digest: string | null | undefined): string {
  if (!digest) return '—'
  const hex = digest.includes(':') ? digest.slice(digest.indexOf(':') + 1) : digest
  return hex.slice(0, 12)
}

/** What the container runs: version label, else the digest it was pulled by, else its image id. */
export function runningText(s: ServiceUpdate): string {
  const r = s.running
  if (r === null) return 'no container'
  const digest = shortDigest(r.repo_digest ?? r.image_id)
  return r.version ? `${r.version} (${digest})` : digest
}

/** What the tag points to now, when it differs. */
export function availableText(s: ServiceUpdate): string | null {
  const a = s.available
  if (a === null) return null
  const digest = shortDigest(a.digest)
  return a.version ? `${a.version} (${digest})` : digest
}

/** Whether a service's reason needs reading: excluded, held, failed and can't-check ones. */
export function reasonOf(s: ServiceUpdate): string | null {
  return s.reason && s.reason.trim() !== '' ? s.reason : null
}

export function authText(a: RegistryAuth): string {
  switch (a.kind) {
    case 'anonymous':
      return 'anonymous'
    case 'config':
      return 'credentials from config.json'
    case 'unsupported':
      return `can't authenticate: ${a.reason}`
  }
}

/** Intervals offered when turning checks on; the server refuses less than an hour. */
export const INTERVALS: readonly { secs: number; label: string }[] = [
  { secs: 3600, label: 'every hour' },
  { secs: 6 * 3600, label: 'every 6 hours' },
  { secs: 12 * 3600, label: 'every 12 hours' },
  { secs: 24 * 3600, label: 'every day' },
]

export function intervalText(secs: number): string {
  const known = INTERVALS.find((i) => i.secs === secs)
  if (known) return known.label
  if (secs % 3600 === 0) return `every ${String(secs / 3600)} hours`
  return `every ${String(secs)} seconds`
}

/** The settings with checks turned on or off, and (when given) a new interval. */
export function withChecks(s: UpdateSettings, on: boolean, intervalSecs?: number): UpdateSettings {
  return { ...s, checks_enabled: on, interval_secs: intervalSecs ?? s.interval_secs }
}

/** The Updates page exists for this person: an admin on a server with updates on. */
export function showUpdates(me: Me | null): boolean {
  return me !== null && me.role === 'admin' && me.features.updates === true
}

/** Why a service is never checked, in words. */
export function exclusionText(e: Exclusion): string {
  switch (e.kind) {
    case 'pinned_by_digest':
      return 'pinned by digest: the image never changes'
    case 'build_context':
      return 'built from a local build context'
    case 'local_image':
      return 'a local image with no registry'
    case 'bad_reference':
      return "the image reference can't be parsed"
    case 'label_disabled':
      return 'turned off by a com.centurylinklabs.watchtower.enable=false label'
    case 'label_only_mode':
      return 'label-only mode is on and this service has no enable label'
    case 'scope_mismatch':
      return e.scope === null
        ? 'outside the configured scope (no scope label)'
        : `outside the configured scope (scope ${e.scope})`
    case 'no_pull':
      return 'pull_policy never: Depths does not pull it'
    case 'depths_itself':
      return "Depths' own container: Depths never updates itself"
  }
}

/** Depths' own container: update it by hand (FR-024). */
export const SELF_UPDATE_STEPS: readonly string[] = ['docker compose pull depths', 'docker compose up -d depths']

export function isSelf(s: ServiceUpdate): boolean {
  return s.effective.excluded?.kind === 'depths_itself'
}

/** `in 3 min`, `in 2 h`: the time until `iso`, from `now`. */
export function untilText(iso: string, now: number = Date.now()): string {
  const ms = new Date(iso).getTime() - now
  if (Number.isNaN(ms)) return iso
  if (ms <= 60_000) return 'in under a minute'
  const min = Math.round(ms / 60_000)
  if (min < 90) return `in ${String(min)} min`
  return `in ${String(Math.round(min / 60))} h`
}

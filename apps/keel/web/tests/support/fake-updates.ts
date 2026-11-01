/**
 * The update notice and self-update on the fake backend (spec 061, contracts/protocol.md),
 * answered by role as apps/keel/server/src/dispatch.rs will:
 *
 * - `GetKeelBuild` answers `KeelBuild` for anyone signed in. Users get the version and `display`
 *   only: `revision`, `built_at` and `digest` are `null` (FR-002).
 * - `GetUpdateStatus` answers `UpdateStatus` to an owner and `UpdateStatusLine` to an admin; a user
 *   gets `Denied { reason: "admin_only" }`.
 * - `CheckForUpdates` (owners): `update_check_off` when checks are off, `update_check_too_soon`
 *   (with `manual_after` as the message) when `tooSoon` is set, else `UpdateStatus` with
 *   `state: "checking"`. The test ends the check with `finishCheck()`, which pushes the result.
 * - `DismissUpdateDot` (owners): `invalid_digest` unless `sha256:<64 hex>`, else `Me` with
 *   `update_dot_dismissed` set.
 * - `PreviewKeelUpdate` (owners): `refusal` if set, then the manual path's `missing` code, then
 *   `digest_stale` (not the available digest, or an outcome with nothing to update to); else a
 *   step-up `Preview`. Its `Confirm` needs a fresh check, then answers `Confirmed` and `Run`: the
 *   new update run, or the one already in progress (FR-018).
 *
 * Anyone else asking an owner-only command gets `Denied { reason: "owner_only" }`. Pushes follow
 * the server's rule (R5): owners get `UpdateStatus`, admins `UpdateStatusLine`, users nothing.
 *
 * Fixtures: `updateFixtures.status(outcome)` for each `UpdateOutcome`, and `manualPath(missing)` for
 * each FR-021 reason. Nothing reads the clock.
 */
import type {
  AvailableImage,
  Cmd,
  Event,
  KeelBuild,
  Me,
  RunInfo,
  UpdateOutcome,
  UpdatePath,
  UpdateStatus,
  UpdateStatusLine,
} from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { NOW } from './fake-repos'
import { SYSTEM_WORKSPACE } from './fake-settings'

export const RUNNING_DIGEST = `sha256:${'1'.repeat(64)}`
export const AVAILABLE_DIGEST = `sha256:${'2'.repeat(64)}`
export const CHECKED_AT = '2026-10-28T12:00:00Z'
export const NEXT_CHECK_AT = '2026-10-28T18:00:00Z'
export const MANUAL_AFTER = '2026-10-28T12:01:00Z'
export const IMAGE = 'docker.io/mbround18/keel:latest'
const EXPIRES = '2026-10-28T12:15:00Z'
const DIGEST = /^sha256:[0-9a-f]{64}$/

/** Every reason the update can't run as a Keel run (FR-021). */
export const MANUAL_REASONS = ['no_system_workspace', 'no_keel_host', 'profile_not_allowed', 'job_missing_param'] as const
export type ManualReason = (typeof MANUAL_REASONS)[number]

/** Outcomes `PreviewKeelUpdate` accepts: there is a different image to move to. */
const UPDATABLE: readonly UpdateOutcome[] = ['update_available', 'rebuilt', 'older_build']

export const updateFixtures = {
  running: (over: Partial<KeelBuild> = {}): KeelBuild => ({
    version: '0.2.0',
    revision: 'abc1234def5678',
    built_at: '2026-10-01T12:00:00Z',
    digest: RUNNING_DIGEST,
    display: '0.2.0',
    ...over,
  }),

  available: (over: Partial<AvailableImage> = {}): AvailableImage => ({
    digest: AVAILABLE_DIGEST,
    version: '0.3.0',
    revision: 'fedcba9876543',
    built_at: '2026-10-27T12:00:00Z',
    release_notes: 'https://github.com/mbround18/fathom/releases/tag/keel-v0.3.0',
    release_notes_note: null,
    ...over,
  }),

  manualPath: (missing: ManualReason = 'no_system_workspace', digest = AVAILABLE_DIGEST): UpdatePath => ({
    kind: 'manual',
    missing,
    digest_ref: `mbround18/keel@${digest}`,
    steps: [`docker pull mbround18/keel@${digest}`, 'docker compose up -d keel'],
  }),

  /**
   * A checked status for `outcome`, shaped as the server builds it: `available` only when there is
   * an image to compare, `cant_tell_reason` only for `cant_tell`.
   */
  status: (outcome: UpdateOutcome = 'update_available', over: Partial<UpdateStatus> = {}): UpdateStatus => {
    const available: Record<UpdateOutcome, AvailableImage | null> = {
      up_to_date: updateFixtures.available({ digest: RUNNING_DIGEST, version: '0.2.0', revision: 'abc1234def5678', built_at: '2026-10-01T12:00:00Z' }),
      update_available: updateFixtures.available(),
      older_build: updateFixtures.available({ version: '0.1.9', revision: '0001111222333', built_at: '2026-09-01T12:00:00Z' }),
      rebuilt: updateFixtures.available({ version: '0.2.0', built_at: '2026-10-20T12:00:00Z' }),
      cant_tell: updateFixtures.available({ version: null, revision: null, built_at: null, release_notes: null, release_notes_note: 'The image has no version label.' }),
    }
    return {
      enabled: true,
      image: IMAGE,
      state: 'checked',
      outcome,
      cant_tell_reason: outcome === 'cant_tell' ? 'no_revision_label' : null,
      checked_at: CHECKED_AT,
      next_check_at: NEXT_CHECK_AT,
      manual_after: MANUAL_AFTER,
      last_error: null,
      running: updateFixtures.running(),
      available: available[outcome],
      digest_compare: true,
      update_path: { kind: 'run' },
      update_run: null,
      last_update_run: null,
      ...over,
    }
  },

  /** Checks are off (FR-004): nothing has been or will be checked. */
  off: (): UpdateStatus => ({
    ...updateFixtures.status('up_to_date'),
    enabled: false,
    state: 'not_checked',
    outcome: null,
    checked_at: null,
    next_check_at: null,
    manual_after: null,
    available: null,
  }),

  /** On but never checked yet. */
  notChecked: (): UpdateStatus => ({
    ...updateFixtures.off(),
    enabled: true,
    next_check_at: NEXT_CHECK_AT,
  }),

  /** The last check failed: the previous result, if any, stays. */
  failed: (code = 'network', over: Partial<UpdateStatus> = {}): UpdateStatus => ({
    ...updateFixtures.status('up_to_date'),
    last_error: { code, at: CHECKED_AT },
    ...over,
  }),

  /** The update run that `Confirm` queues. */
  run: (id: string, digest: string): RunInfo => ({
    id,
    workspace: SYSTEM_WORKSPACE,
    binding: 'binding-system',
    job: 'job-keel-server',
    job_name: 'keel-server',
    trigger: 'manual',
    requested_by: null,
    commit: 'c'.repeat(40),
    inventory: 'keel-system',
    inventory_revision: null,
    newer_inventory_revision: false,
    limit: null,
    hosts: ['keel-1'],
    policy: null,
    state: 'queued',
    reason: null,
    created_at: NOW,
    started_at: null,
    ended_at: null,
    summary: null,
    params: { keel_server_image_digest: digest },
    approval: null,
  }),
}

/** The admin line for a status: nothing about the image (FR-015). */
export function statusLine(s: UpdateStatus): UpdateStatusLine {
  return { update_available: s.outcome === 'update_available', checked_at: s.checked_at }
}

/** The running build as `role` sees it. */
export function buildFor(role: Me['instance_role'], b: KeelBuild): KeelBuild {
  return role === 'user' ? { ...b, revision: null, built_at: null, digest: null } : b
}

export class FakeUpdates {
  /** The owner's view; admins and users get it reshaped. */
  status: UpdateStatus = updateFixtures.off()
  /** `CheckForUpdates` answers `update_check_too_soon` while set. */
  tooSoon = false
  /** Answers `PreviewKeelUpdate` with this `OpError` code (e.g. `no_keel_host`), before any check. */
  refusal: string | null = null
  /** How many `CheckForUpdates` were accepted. */
  checks = 0

  private readonly pending = new Map<string, string>()
  private seq = 0
  private readonly backend: FakeBackend

  constructor(backend: FakeBackend) {
    this.backend = backend
  }

  /** Ends a check: the status becomes `next` (checked by default) and is pushed by role. */
  finishCheck(next: UpdateStatus = updateFixtures.status()): void {
    this.status = next
    this.push()
  }

  /** Sends the current status to the open sockets, shaped by the session's role (R5). */
  push(): void {
    const e = this.statusEvent(this.backend.me)
    if (e) this.backend.push(e)
  }

  /** Answers the five update commands and their `Confirm`; `undefined` for anything else. */
  respond(me: Me, cmd: Cmd): Event[] | undefined {
    const owner = me.instance_role === 'owner'
    switch (cmd.type) {
      case 'GetKeelBuild':
        return [{ type: 'KeelBuild', data: buildFor(me.instance_role, this.status.running) }]
      case 'GetUpdateStatus':
        return [this.statusEvent(me) ?? denied('get_update_status', 'admin_only')]
      case 'CheckForUpdates': {
        if (!owner) return [denied('check_for_updates', 'owner_only')]
        if (!this.status.enabled) return error('update_check_off', 'Update checks are off (KEEL_UPDATE_CHECK).')
        if (this.tooSoon) return error('update_check_too_soon', this.status.manual_after ?? MANUAL_AFTER)
        this.checks++
        this.status = { ...this.status, state: 'checking' }
        return [{ type: 'UpdateStatus', data: this.status }]
      }
      case 'DismissUpdateDot': {
        if (!owner) return [denied('dismiss_update_dot', 'owner_only')]
        const { digest } = cmd.data
        if (!DIGEST.test(digest)) return error('invalid_digest', 'That is not an image digest.')
        const next = { ...me, update_dot_dismissed: digest }
        this.backend.me = next
        return [{ type: 'Me', data: next }]
      }
      case 'PreviewKeelUpdate':
        return owner ? this.preview(cmd.data.digest) : [denied('preview_keel_update', 'owner_only')]
      case 'Confirm': {
        const { preview_id, step_up } = cmd.data
        const digest = this.pending.get(preview_id)
        if (digest === undefined) return undefined
        this.pending.delete(preview_id)
        if (!step_up && !me.verification.verified) {
          return error('step_up_required', 'Confirm it’s you with a passkey first.')
        }
        const id = this.status.update_run ?? `run-update-${String(++this.seq).padStart(4, '0')}`
        this.status = { ...this.status, update_run: id }
        return [
          { type: 'Confirmed', data: { preview_id } },
          { type: 'Run', data: { run: updateFixtures.run(id, digest), hosts: [] } },
        ]
      }
      default:
        return undefined
    }
  }

  private statusEvent(me: Me | null): Event | null {
    switch (me?.instance_role) {
      case 'owner':
        return { type: 'UpdateStatus', data: this.status }
      case 'admin':
        return { type: 'UpdateStatusLine', data: statusLine(this.status) }
      default:
        return null
    }
  }

  private preview(digest: string): Event[] {
    if (this.refusal !== null) return error(this.refusal, `The update can’t run as a Keel run (${this.refusal}).`)
    const s = this.status
    if (s.update_path.kind === 'manual') {
      return error(s.update_path.missing, `The update can’t run as a Keel run (${s.update_path.missing}).`)
    }
    if (s.available?.digest !== digest || s.outcome === null || !UPDATABLE.includes(s.outcome)) {
      return error('digest_stale', 'The available image changed. Check again.')
    }
    const preview_id = `update-preview-${String(++this.seq)}`
    this.pending.set(preview_id, digest)
    // contracts/protocol.md: old and new version, revision and digest; workspace, host and job; the
    // approval policy; a warning line first for older_build and rebuilt.
    const side = (version: string | null, revision: string | null, d: string | null) =>
      `${version ?? 'no version'} (${revision?.slice(0, 7) ?? 'no revision'}, ${d?.slice(0, 19) ?? 'no digest'})`
    const warning: Partial<Record<UpdateOutcome, string>> = {
      older_build: 'Warning: the watched image is an older build; this goes back. ',
      rebuilt: 'Warning: the watched image is a rebuild of the running revision. ',
    }
    const summary =
      s.update_run === null
        ? `${warning[s.outcome] ?? ''}Update Keel from ${side(s.running.version, s.running.revision, s.running.digest)} ` +
          `to ${side(s.available.version, s.available.revision, digest)} by running job keel-server on keel-1 ` +
          `in the System workspace (inventory keel-system). No approval policy applies. Keel restarts.`
        : `An update run is already in progress; confirming opens it.`
    return [{ type: 'Preview', data: { preview_id, action: 'keel_update', summary, affected: ['keel-1'], step_up: true, expires_at: EXPIRES } }]
  }
}

function denied(action: string, reason: string): Event {
  return { type: 'Denied', data: { action, reason } }
}

function error(code: string, message: string): Event[] {
  return [{ type: 'OpError', data: { code, message } }]
}

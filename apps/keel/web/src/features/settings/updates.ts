/**
 * The running build and the update notice (spec 061, contracts/protocol.md).
 *
 * - `GetKeelBuild` answers the running build, shaped by role; the System tab's build panel reads it.
 * - `GetUpdateStatus` is sent once `Me` says the person is an owner or an admin, and again on
 *   reconnect. Owners get `UpdateStatus`, admins `UpdateStatusLine`. The server also pushes either
 *   whenever the status changes; every one that arrives replaces what is held, answer or push.
 * - `CheckForUpdates`, `DismissUpdateDot` and `PreviewKeelUpdate` are the owner's actions. The
 *   update goes through the governance preview → confirm dialog, with its passkey step-up.
 *
 * The update run: the status names a pending, queued or running one (`update_run`). When that goes
 * back to `null`, the run is read once more, so a failed update can say so with a link to it.
 *
 * Kept apart from ./store.ts so the shell can draw the navigation dot without loading the rest of
 * System Settings.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { useGovernance, type Refusal } from '@/features/teams/store'
import type {
  Cmd,
  Event,
  KeelBuild,
  Me,
  RequestId,
  RunInfo,
  RunPhase,
  UpdateOutcome,
  UpdateStatus,
  UpdateStatusLine,
} from '@/protocol/wire'

/** RFC 3339 to minutes, UTC: `2026-10-28 14:05 UTC`. */
export function when(rfc3339: string): string {
  return `${rfc3339.slice(0, 16).replace('T', ' ')} UTC`
}

/** `sha256:` and the first 12 hex characters. */
export function shortDigest(digest: string): string {
  return digest.slice(0, 'sha256:'.length + 12)
}

/** A revision's first 7 characters, keeping a `-dirty` suffix. */
export function shortRevision(revision: string): string {
  const dirty = revision.endsWith('-dirty')
  return revision.slice(0, 7) + (dirty ? '-dirty' : '')
}

/** What an admin who isn't an owner reads when an update is available (FR-015). */
export const ADMIN_LINE = 'A newer Keel is available; an owner can update it.'

/**
 * Whether the System Settings entry shows the update dot (data-model.md "Dot dismissal"):
 * `role == owner && status.outcome == update_available && status.available.digest != me.update_dot_dismissed`.
 */
export function showDot(me: Me | null, status: UpdateStatus | null): boolean {
  return (
    me?.instance_role === 'owner' &&
    status?.outcome === 'update_available' &&
    status.available?.digest !== me.update_dot_dismissed
  )
}

/** The admin's read-only line, or `null` when there is nothing to say (FR-015). */
export function adminLine(line: UpdateStatusLine | null): string | null {
  return line?.update_available === true ? ADMIN_LINE : null
}

/** What the owner's card shows first: whether checks run, then where the last one stands. */
export type CardState = 'off' | 'not_checked' | 'checking' | UpdateOutcome

export function cardState(s: UpdateStatus): CardState {
  if (!s.enabled) return 'off'
  if (s.state === 'checking') return 'checking'
  return s.outcome ?? 'not_checked'
}

/** Outcomes with a different image to move to: what `PreviewKeelUpdate` accepts. */
export function canUpdate(s: UpdateStatus): boolean {
  return s.enabled && s.available !== null && (s.outcome === 'update_available' || s.outcome === 'rebuilt' || s.outcome === 'older_build')
}

/** Run states that end an update without it having happened. */
const UNSUCCESSFUL: readonly RunPhase[] = ['failed', 'refused', 'cancelled', 'denied', 'expired']

export function runFailed(run: RunInfo | null): boolean {
  return run !== null && UNSUCCESSFUL.includes(run.state)
}

export interface Held {
  status: UpdateStatus | null
  line: UpdateStatusLine | null
}

/** Folds an event into what is held: `UpdateStatus` and `UpdateStatusLine` replace it; anything else leaves it. */
export function reduce(held: Held, event: Event): Held {
  if (event.type === 'UpdateStatus') return { status: event.data, line: null }
  if (event.type === 'UpdateStatusLine') return { status: null, line: event.data }
  return held
}

export interface UpdatesState extends Held {
  build: KeelBuild | null
  buildRefusal: Refusal | null
  /** Why `GetUpdateStatus` was refused, if it was. */
  refusal: Refusal | null
  /** Why the last Check now or dismissal was refused (`update_check_too_soon` carries the retry time). */
  actionRefusal: Refusal | null
  /** The last update run this page saw: started here, or named by the status. */
  run: RunInfo | null

  loadBuild: () => void
  loadStatus: () => void
  checkNow: () => void
  dismissDot: (digest: string) => void
  update: (digest: string) => void
}

const OFFLINE: Refusal = {
  kind: 'error',
  code: 'offline',
  message: 'Keel is not connected. Wait for it to reconnect, then try again.',
}

const handlers = new Map<RequestId, (event: Event) => void>()
let askedBuild = false
let askedStatus = false

function refusalOf(event: Event): Refusal | null {
  if (event.type === 'Denied') return { kind: 'denied', action: event.data.action, reason: event.data.reason }
  if (event.type === 'OpError') return { kind: 'error', code: event.data.code, message: event.data.message }
  return null
}

/** Sends `cmd`; `handle` sees each answer until it returns `true`. `false` when offline. */
function ask(cmd: Cmd, handle: (event: Event) => boolean): boolean {
  const id = sendCmd(cmd)
  if (id === null) return false
  handlers.set(id, (event) => {
    if (handle(event)) handlers.delete(id)
  })
  return true
}

/** Reads a run once, for its final state. */
function readRun(run: string): void {
  ask({ type: 'GetRun', data: { run } }, (event) => {
    if (event.type === 'Run' && event.data.run.id === run) {
      useUpdates.setState({ run: event.data.run })
      return true
    }
    return refusalOf(event) !== null
  })
}

function applyStatus(event: Event): void {
  const before = useUpdates.getState()
  const held = reduce(before, event)
  if (held === before) return
  useUpdates.setState({ ...held, refusal: null })
  const was = before.status?.update_run ?? null
  const now = held.status?.update_run ?? null
  // The update run ended: read it once more, so a failure can say so.
  if (was !== null && now === null) readRun(was)
  else if (now !== null && now !== was && useUpdates.getState().run?.id !== now) readRun(now)
}

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined) handlers.get(reply_to)?.(event)
  applyStatus(event)
})

onReconnected(() => {
  const s = useUpdates.getState()
  if (askedBuild) s.loadBuild()
  if (askedStatus) s.loadStatus()
})


export const useUpdates = create<UpdatesState>()((set) => ({
  status: null,
  line: null,
  build: null,
  buildRefusal: null,
  refusal: null,
  actionRefusal: null,
  run: null,

  loadBuild: () => {
    askedBuild = true
    const sent = ask({ type: 'GetKeelBuild' }, (event) => {
      if (event.type === 'KeelBuild') {
        set({ build: event.data, buildRefusal: null })
        return true
      }
      const refusal = refusalOf(event)
      if (refusal) set({ buildRefusal: refusal })
      return refusal !== null
    })
    if (!sent) set({ buildRefusal: OFFLINE })
  },

  loadStatus: () => {
    askedStatus = true
    // The answer is an `UpdateStatus` or `UpdateStatusLine`, which every event handler applies.
    const sent = ask({ type: 'GetUpdateStatus' }, (event) => {
      if (event.type === 'UpdateStatus' || event.type === 'UpdateStatusLine') return true
      const refusal = refusalOf(event)
      if (refusal) set({ refusal })
      return refusal !== null
    })
    if (!sent) set({ refusal: OFFLINE })
  },

  checkNow: () => {
    set({ actionRefusal: null })
    const sent = ask({ type: 'CheckForUpdates' }, (event) => {
      if (event.type === 'UpdateStatus') return true
      const refusal = refusalOf(event)
      if (refusal) set({ actionRefusal: refusal })
      return refusal !== null
    })
    if (!sent) set({ actionRefusal: OFFLINE })
  },

  dismissDot: (digest) => {
    set({ actionRefusal: null })
    // The answer is `Me`, which the session store takes.
    const sent = ask({ type: 'DismissUpdateDot', data: { digest } }, (event) => {
      if (event.type === 'Me') return true
      const refusal = refusalOf(event)
      if (refusal) set({ actionRefusal: refusal })
      return refusal !== null
    })
    if (!sent) set({ actionRefusal: OFFLINE })
  },

  update: (digest) => {
    useGovernance.getState().propose('Update Keel', { type: 'PreviewKeelUpdate', data: { digest } }, (event) => {
      // `Confirm` answers the new run, or the one already in progress (FR-018).
      if (event.type === 'Run') set({ run: event.data.run })
      else if (event.type === 'Confirmed') useUpdates.getState().loadStatus()
    })
  },
}))

function seesUpdates(me: Me | null): boolean {
  return me?.instance_role === 'owner' || me?.instance_role === 'admin'
}

// The status is read once `Me` says who this is, and again if their role changes.
useSession.subscribe((s, prev) => {
  if (s.me?.instance_role === prev.me?.instance_role) return
  if (seesUpdates(s.me)) useUpdates.getState().loadStatus()
  else useUpdates.setState({ status: null, line: null, refusal: null, run: null })
})
// Loaded after `Me` already arrived.
if (seesUpdates(useSession.getState().me)) useUpdates.getState().loadStatus()

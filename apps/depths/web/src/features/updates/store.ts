/**
 * The Updates page (spec 062) over the shared socket.
 *
 * - `load()` sends `ListUpdates`; the reply is `Updates`, then the server pushes `UpdatesChanged`
 *   (coalesced, at most one a second) for as long as the socket lives.
 * - `checkNow()` sends `CheckUpdatesNow`: `CheckStarted`, `CheckBusy{retry_after_secs}`, or
 *   `Invalid` while checks are off.
 * - `previewChange(c)` sends `PreviewUpdateChange(c)` (settings, a stack policy, a Watchtower
 *   acknowledgement, a retry or an apply-anyway); the reply is a `Preview` whose `file_diff` is the
 *   before/after summary, always with `step_up`. `confirm()` sends its `preview_id` with the passkey
 *   assertion (054). A cancelled prompt, `StepUpRequired` or `PasskeyRefused` keep the preview open
 *   with `note`; the next Confirm asks again. A change answers `UpdatesChanged`; an apply-anyway
 *   starts a one-stack run (`UpdateRunStarted`), which the run view follows.
 * - US2: `previewBatch()` sends `PreviewUpdateBatch` for the selected stacks; `confirmBatch()`
 *   confirms it (acknowledging 051 findings when asked to). The reply is `UpdateRunStarted`; the
 *   rest of the run is pushed without a request id: per stack `OpStarted`, `OpOutput`,
 *   `OpFinished` and `UpdateStackDone`, then `UpdateRunFinished`. `OpStarted` names no stack, so
 *   each one belongs to the next stack in the run's order that has not started.
 * - US3/US5: `loadRunAudit()` reads the `image_remove` and `image_rollback` audit entries: the
 *   recent runs show the images each run removed and the output of a failed rollback.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { stepUpAssertion } from '@/features/passkeys/api'
import { stepUpText } from '@/features/passkeys/messages'
import type {
  AuditEntry,
  Event,
  PasskeyAssertion,
  PasskeyRequest,
  Preview,
  RequestId,
  RunTrigger,
  StackOutcome,
  StackUpdatePlan,
  UpdateChange,
  UpdateSettings,
  UpdatesView,
} from '@/protocol/generated'

import { DENY_TEXT } from '@/features/stacks/argv'

import type { StateFilter } from './model'

export type CheckStatus =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'started' }
  | { kind: 'busy'; retryAfterSecs: number }
  | { kind: 'refused'; reason: string }

/** 054: the passkey request the next Confirm answers (`null` once spent), and why it asks again. */
interface StepUp {
  stepUp: PasskeyRequest | null
  note: string | null
}

export type ChangeFlow =
  | { kind: 'idle' }
  | { kind: 'previewing'; change: UpdateChange }
  | ({ kind: 'preview' | 'verifying' | 'confirming'; change: UpdateChange; preview: Preview } & StepUp)
  | { kind: 'refused'; change: UpdateChange; reason: string }

export interface BatchPreview {
  previewId: string
  stacks: StackUpdatePlan[]
  ackRequired: boolean
}

export type BatchFlow =
  | { kind: 'idle' }
  | { kind: 'previewing'; stacks: string[] }
  | ({ kind: 'preview' | 'verifying' | 'confirming'; preview: BatchPreview; acknowledged: boolean } & StepUp)
  /** No service would be recreated: information, with each stack's reasons, not a refusal. */
  | { kind: 'up_to_date'; stacks: StackUpdatePlan[] }
  | { kind: 'refused'; reason: string }

export interface RunStack {
  stack: string
  operationId: string | null
  output: string
  outcome: StackOutcome | null
}

/** A run this page started, followed live until its summary. */
export interface LiveRun {
  runId: string
  trigger: RunTrigger
  stacks: RunStack[]
  finished: boolean
  cancelling: boolean
}

export interface UpdatesState {
  view: UpdatesView | null
  loaded: boolean
  /** `Denied`: not an admin, or the feature is off. */
  denied: string | null
  stateFilter: StateFilter
  stackFilter: string
  check: CheckStatus
  change: ChangeFlow
  /** Stacks chosen for the next batch. */
  selected: string[]
  batch: BatchFlow
  run: LiveRun | null
  /** The audit entries the recent runs show, by action, newest first; empty until asked. */
  runAudit: Partial<Record<RunAuditAction, AuditEntry[]>>
  load: () => void
  setStateFilter: (f: StateFilter) => void
  setStackFilter: (s: string) => void
  checkNow: () => void
  previewChange: (change: UpdateChange) => void
  previewSettings: (after: UpdateSettings) => void
  confirm: () => void
  cancel: () => void
  toggleStack: (stack: string) => void
  setSelected: (stacks: string[]) => void
  previewBatch: () => void
  acknowledge: (on: boolean) => void
  confirmBatch: () => void
  cancelBatch: () => void
  stopRun: () => void
  closeRun: () => void
  loadRunAudit: () => void
}

export const RUN_AUDIT_ACTIONS = ['image_remove', 'image_rollback'] as const
export type RunAuditAction = (typeof RUN_AUDIT_ACTIONS)[number]

type Asked =
  | 'list'
  | 'check'
  | 'preview'
  | 'confirm'
  | 'batch-preview'
  | 'batch-confirm'
  | 'cancel'
  | `audit:${RunAuditAction}`
const pending = new Map<RequestId, Asked>()
let auditWanted = false
let wanted = false

/** Output kept per stack; older text is dropped from the front. */
const OUTPUT_CAP = 64 * 1024

const INITIAL = {
  view: null,
  loaded: false,
  denied: null,
  stateFilter: 'all' as StateFilter,
  stackFilter: '',
  check: { kind: 'idle' } as CheckStatus,
  change: { kind: 'idle' } as ChangeFlow,
  selected: [] as string[],
  batch: { kind: 'idle' } as BatchFlow,
  run: null as LiveRun | null,
  runAudit: {} as Partial<Record<RunAuditAction, AuditEntry[]>>,
}

function send(asked: Asked, cmd: Parameters<typeof sendCmd>[0]): boolean {
  const id = sendCmd(cmd)
  if (id === null) return false
  pending.set(id, asked)
  return true
}

/**
 * The 054 confirm: ask for the passkey when the preview carries a request, then `go(assertion)`.
 * `current()` re-reads the flow, so a dialog closed during the prompt sends nothing.
 */
function withPasskey<F extends StepUp & { kind: string }>(
  start: F,
  current: () => F | null,
  set: (f: F) => void,
  go: (assertion?: PasskeyAssertion) => void,
): void {
  const request = start.stepUp
  if (!request) {
    go()
    return
  }
  set({ ...start, kind: 'verifying', note: null })
  void stepUpAssertion(request).then((r) => {
    const flow = current()
    if (flow?.kind !== 'verifying') return
    if (r.ok) go(r.assertion)
    else set({ ...flow, kind: 'preview', note: stepUpText(r.refusal) })
  })
}

export const useUpdates = create<UpdatesState>()((set, get) => ({
  ...INITIAL,

  load: () => {
    wanted = true
    send('list', { type: 'ListUpdates' })
  },

  setStateFilter: (stateFilter) => set({ stateFilter }),
  setStackFilter: (stackFilter) => set({ stackFilter }),

  checkNow: () => {
    if (get().check.kind === 'sending') return
    set({ check: send('check', { type: 'CheckUpdatesNow', data: {} }) ? { kind: 'sending' } : { kind: 'idle' } })
  },

  previewChange: (change) => {
    const ok = send('preview', { type: 'PreviewUpdateChange', data: change })
    set({ change: ok ? { kind: 'previewing', change } : { kind: 'refused', change, reason: 'Not connected.' } })
  },

  previewSettings: (after) => get().previewChange({ kind: 'settings', data: after }),

  confirm: () => {
    const start = get().change
    if (start.kind !== 'preview') return
    type Live = Extract<ChangeFlow, { preview: Preview }>
    const current = (): Live | null => {
      const f = get().change
      return 'preview' in f && f.preview === start.preview ? f : null
    }
    withPasskey<Live>(
      start,
      current,
      (change) => set({ change }),
      (assertion) => {
        const flow = current()
        if (!flow) return
        const ok = send('confirm', {
          type: 'Confirm',
          data: { preview_id: start.preview.preview_id, ...(assertion ? { assertion } : {}) },
        })
        set({
          change: ok
            ? { ...flow, kind: 'confirming', stepUp: null, note: null }
            : { kind: 'refused', change: flow.change, reason: 'Not connected.' },
        })
      },
    )
  },

  cancel: () => set({ change: { kind: 'idle' } }),

  toggleStack: (stack) => {
    const { selected } = get()
    set({ selected: selected.includes(stack) ? selected.filter((s) => s !== stack) : [...selected, stack].sort() })
  },

  setSelected: (stacks) => set({ selected: [...new Set(stacks)].sort() }),

  previewBatch: () => {
    const stacks = get().selected
    if (stacks.length === 0) return
    const ok = send('batch-preview', { type: 'PreviewUpdateBatch', data: { stacks } })
    set({ batch: ok ? { kind: 'previewing', stacks } : { kind: 'refused', reason: 'Not connected.' } })
  },

  acknowledge: (on) => {
    const b = get().batch
    if (b.kind === 'preview') set({ batch: { ...b, acknowledged: on } })
  },

  confirmBatch: () => {
    const start = get().batch
    if (start.kind !== 'preview') return
    if (start.preview.ackRequired && !start.acknowledged) return
    type Live = Extract<BatchFlow, { preview: BatchPreview }>
    const current = (): Live | null => {
      const f = get().batch
      return 'preview' in f && f.preview === start.preview ? f : null
    }
    withPasskey<Live>(
      start,
      current,
      (batch) => set({ batch }),
      (assertion) => {
        const flow = current()
        if (!flow) return
        const ok = send('batch-confirm', {
          type: 'Confirm',
          data: {
            preview_id: start.preview.previewId,
            ...(start.acknowledged ? { acknowledge_findings: true } : {}),
            ...(assertion ? { assertion } : {}),
          },
        })
        set({ batch: ok ? { ...flow, kind: 'confirming', stepUp: null, note: null } : { kind: 'refused', reason: 'Not connected.' } })
      },
    )
  },

  cancelBatch: () => set({ batch: { kind: 'idle' } }),

  stopRun: () => {
    const run = get().run
    if (!run || run.finished || run.cancelling) return
    const current = run.stacks.find((s) => s.operationId !== null && s.outcome === null)
    if (!current?.operationId) return
    if (send('cancel', { type: 'Cancel', data: { operation_id: current.operationId } })) set({ run: { ...run, cancelling: true } })
  },

  closeRun: () => {
    const run = get().run
    if (run && !run.finished) return
    set({ run: null })
  },

  loadRunAudit: () => {
    auditWanted = true
    for (const action of RUN_AUDIT_ACTIONS) send(`audit:${action}`, { type: 'GetAudit', data: { filter: { action } } })
  },
}))

function voidText(reason: string): string {
  switch (reason) {
    case 'expired':
      return 'The preview expired. Try again.'
    case 'used':
      return 'That preview was already used.'
    default:
      return 'The preview is no longer valid. Try again.'
  }
}

function deniedText(e: Extract<Event, { type: 'Denied' }>): string {
  return e.data.detail ? `${DENY_TEXT[e.data.reason]} ${e.data.detail}` : DENY_TEXT[e.data.reason]
}

function startRun(e: Extract<Event, { type: 'UpdateRunStarted' }>): void {
  useUpdates.setState({
    run: {
      runId: e.data.run_id,
      trigger: e.data.trigger,
      stacks: e.data.stacks.map((stack) => ({ stack, operationId: null, output: '', outcome: null })),
      finished: false,
      cancelling: false,
    },
  })
}

function updateRun(f: (run: LiveRun) => LiveRun | null): void {
  const run = useUpdates.getState().run
  if (!run) return
  const next = f(run)
  if (next) useUpdates.setState({ run: next })
}

/** Run events with no request id: they belong to the run this page started, if any. */
function onRunEvent(event: Event): void {
  switch (event.type) {
    case 'OpStarted':
      updateRun((run) => {
        if (run.finished) return null
        const i = run.stacks.findIndex((s) => s.operationId === null && s.outcome === null)
        if (i < 0) return null
        const cur = run.stacks[i]
        if (cur === undefined) return null
        const stacks = run.stacks.slice()
        stacks[i] = { ...cur, operationId: event.data.operation_id }
        return { ...run, stacks }
      })
      return
    case 'OpOutput':
      updateRun((run) => {
        const i = run.stacks.findIndex((s) => s.operationId === event.data.operation_id)
        if (i < 0) return null
        const s = run.stacks[i]
        if (s === undefined) return null
        const stacks = run.stacks.slice()
        const output = s.output + event.data.chunk
        stacks[i] = { ...s, output: output.length > OUTPUT_CAP ? output.slice(output.length - OUTPUT_CAP) : output }
        return { ...run, stacks }
      })
      return
    case 'UpdateStackDone':
      updateRun((run) => {
        if (run.runId !== event.data.run_id) return null
        return { ...run, stacks: run.stacks.map((s) => (s.stack === event.data.stack ? { ...s, outcome: event.data.outcome } : s)) }
      })
      return
    case 'UpdateRunFinished':
      updateRun((run) => {
        if (run.runId !== event.data.run_id) return null
        const by = new Map(event.data.summary)
        return {
          ...run,
          finished: true,
          cancelling: false,
          stacks: run.stacks.map((s) => ({ ...s, outcome: by.get(s.stack) ?? s.outcome })),
        }
      })
      // Removals and rollbacks are audited during the run: read them again for the recent runs.
      if (auditWanted) useUpdates.getState().loadRunAudit()
      return
    default:
      return
  }
}

/** The refusals a Confirm can get, for either flow. `null`: not one of them. */
function confirmRefusal<F extends StepUp & { kind: string }>(
  event: Event,
  flow: F,
): { kind: 'again'; flow: F } | { kind: 'refused'; reason: string } | null {
  switch (event.type) {
    case 'StepUpRequired':
      return { kind: 'again', flow: { ...flow, kind: 'preview', stepUp: event.data.request, note: stepUpText({ kind: 'step_up' }) } }
    case 'PasskeyRefused':
      return {
        kind: 'again',
        flow: { ...flow, kind: 'preview', stepUp: null, note: stepUpText({ kind: 'passkey', reason: event.data.reason }) },
      }
    case 'PasskeyEnrolmentRequired':
      return { kind: 'refused', reason: stepUpText({ kind: 'enrol' }) }
    case 'PreviewVoid':
      return { kind: 'refused', reason: voidText(event.data.reason) }
    case 'Invalid':
      return { kind: 'refused', reason: event.data.reason }
    case 'Denied':
      return { kind: 'refused', reason: deniedText(event) }
    default:
      return null
  }
}

onServerEvent(({ reply_to, event }) => {
  // Pushed changes carry no request id.
  if (event.type === 'UpdatesChanged') {
    useUpdates.setState({ view: event.data, loaded: true })
  }
  if (reply_to === undefined) {
    onRunEvent(event)
    return
  }
  const asked = pending.get(reply_to)
  if (!asked) return
  pending.delete(reply_to)
  const s = useUpdates.getState()

  switch (asked) {
    case 'list':
      if (event.type === 'Updates') useUpdates.setState({ view: event.data, loaded: true, denied: null })
      else if (event.type === 'Denied') useUpdates.setState({ loaded: true, denied: event.data.reason })
      return
    case 'check':
      if (event.type === 'CheckStarted') useUpdates.setState({ check: { kind: 'started' } })
      else if (event.type === 'CheckBusy')
        useUpdates.setState({ check: { kind: 'busy', retryAfterSecs: event.data.retry_after_secs } })
      else if (event.type === 'Invalid') useUpdates.setState({ check: { kind: 'refused', reason: event.data.reason } })
      else if (event.type === 'Denied') useUpdates.setState({ check: { kind: 'refused', reason: DENY_TEXT[event.data.reason] } })
      else useUpdates.setState({ check: { kind: 'idle' } })
      return
    case 'preview': {
      const flow = s.change
      if (flow.kind !== 'previewing') return
      if (event.type === 'Preview')
        useUpdates.setState({
          change: { kind: 'preview', change: flow.change, preview: event.data, stepUp: event.data.step_up ?? null, note: null },
        })
      else if (event.type === 'Invalid') useUpdates.setState({ change: { kind: 'refused', change: flow.change, reason: event.data.reason } })
      else if (event.type === 'Denied') useUpdates.setState({ change: { kind: 'refused', change: flow.change, reason: deniedText(event) } })
      else if (event.type === 'PasskeyEnrolmentRequired')
        useUpdates.setState({ change: { kind: 'refused', change: flow.change, reason: stepUpText({ kind: 'enrol' }) } })
      return
    }
    case 'confirm': {
      const flow = s.change
      if (event.type === 'UpdatesChanged') {
        useUpdates.setState({ change: { kind: 'idle' } })
        return
      }
      if (event.type === 'UpdateRunStarted') {
        useUpdates.setState({ change: { kind: 'idle' } })
        startRun(event)
        return
      }
      if (flow.kind !== 'confirming') return
      const r = confirmRefusal(event, flow)
      if (r?.kind === 'again') useUpdates.setState({ change: r.flow })
      else if (r) useUpdates.setState({ change: { kind: 'refused', change: flow.change, reason: r.reason } })
      return
    }
    case 'batch-preview':
      if (s.batch.kind !== 'previewing') return
      if (event.type === 'UpdatePreview')
        useUpdates.setState({
          batch: {
            kind: 'preview',
            preview: {
              previewId: event.data.preview_id,
              stacks: event.data.stacks,
              ackRequired: event.data.stacks.some((p) => p.ack_required),
            },
            acknowledged: false,
            stepUp: event.data.step_up ?? null,
            note: null,
          },
        })
      else if (event.type === 'NothingToUpdate') useUpdates.setState({ batch: { kind: 'up_to_date', stacks: event.data.stacks } })
      else if (event.type === 'Invalid') useUpdates.setState({ batch: { kind: 'refused', reason: event.data.reason } })
      else if (event.type === 'Denied') useUpdates.setState({ batch: { kind: 'refused', reason: deniedText(event) } })
      else if (event.type === 'PasskeyEnrolmentRequired')
        useUpdates.setState({ batch: { kind: 'refused', reason: stepUpText({ kind: 'enrol' }) } })
      return
    case 'batch-confirm': {
      if (event.type === 'UpdateRunStarted') {
        useUpdates.setState({ batch: { kind: 'idle' }, selected: [] })
        startRun(event)
        return
      }
      const flow = s.batch
      if (flow.kind !== 'confirming') return
      const r = confirmRefusal(event, flow)
      if (r?.kind === 'again') useUpdates.setState({ batch: r.flow })
      else if (r) useUpdates.setState({ batch: { kind: 'refused', reason: r.reason } })
      return
    }
    case 'cancel':
      // The stop shows as the stack's outcome; a refusal (already finished) clears the flag.
      if (event.type !== 'OpFinished') updateRun((run) => ({ ...run, cancelling: false }))
      return
    default: {
      const action = asked.slice('audit:'.length) as RunAuditAction
      const entries = event.type === 'AuditPage' ? event.data.entries : []
      useUpdates.setState({ runAudit: { ...s.runAudit, [action]: entries } })
      return
    }
  }
})

onReconnected(() => {
  pending.clear()
  useUpdates.setState({ check: { kind: 'idle' }, change: { kind: 'idle' }, batch: { kind: 'idle' } })
  // A run keeps going on the server; its events went to the old socket. Its summary is in the
  // recent runs once the page reloads.
  updateRun((run) => (run.finished ? null : { ...run, finished: true, cancelling: false }))
  if (wanted) useUpdates.getState().load()
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetUpdatesStore()
    return
  }
  if (s.connection === 'open' && prev.connection !== 'open' && wanted && !useUpdates.getState().loaded) {
    useUpdates.getState().load()
  }
})

/** Test hook. */
export function resetUpdatesStore(): void {
  pending.clear()
  wanted = false
  auditWanted = false
  useUpdates.setState({ ...INITIAL })
}

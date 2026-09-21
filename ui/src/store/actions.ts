/**
 * `runAction` (T093): the confirmed half of the destructive-action flow (data-model.md §4).
 *
 *   dialog confirmed → send cmd, pending[rid] → CommandFinished{ok, error} → toast ✓/✗
 *     → on success: clear the selection and refresh the list
 *
 * There is no optimistic change: rows disappear or update only when the refreshed list arrives
 * from the backend (constitution I). Backend errors are shown verbatim. A command with no result
 * after `ACTION_TIMEOUT_MS` gets a "no response" toast instead of hanging; if its result arrives
 * later it is still reported.
 */
import { toast } from 'sonner'
import type { StoreApi } from 'zustand'

import type { BackendEvent } from '@/protocol/types'
import type { ActionDescriptor, ActionInput, ActionStep, ResourceDescriptor } from '@/resources'

import { useStore, type PaneId, type StoreState } from './index'

/** The payload of `BackendEvent::CommandFinished` (correlated by `Correlation::Request`). */
export type CommandResult = Extract<BackendEvent, { type: 'CommandFinished' }>['data']

export const ACTION_TIMEOUT_MS = 30_000
let timeoutMs = ACTION_TIMEOUT_MS

/** Tests: shorten (or restore) the no-response timeout. */
export function setActionTimeout(ms: number = ACTION_TIMEOUT_MS): void {
  timeoutMs = ms
}

/** Resolvers for requests awaiting their `CommandFinished`, by request id. */
const waiters = new Map<number, (r: CommandResult) => void>()
/**
 * Results that arrived before `send` resolved with their id (the event channel can beat the
 * invoke reply), or for commands nobody waits on (refreshes). Bounded, oldest dropped first.
 */
const early = new Map<number, CommandResult>()
const EARLY_LIMIT = 512

/** Called by the reducer for every `CommandFinished`. */
export function commandFinished(requestId: number, result: CommandResult): void {
  const waiter = waiters.get(requestId)
  if (waiter) {
    waiters.delete(requestId)
    waiter(result)
    return
  }
  early.set(requestId, result)
  if (early.size > EARLY_LIMIT) {
    const oldest = early.keys().next()
    if (!oldest.done) early.delete(oldest.value)
  }
}

/** Tests: forget every waiter and early result. */
export function resetActions(): void {
  waiters.clear()
  early.clear()
}

type StepResult =
  { status: 'ok' } | { status: 'failed'; error: string } | { status: 'timeout'; requestId: number }

function waitFor(requestId: number): Promise<CommandResult | 'timeout'> {
  const done = early.get(requestId)
  if (done) {
    early.delete(requestId)
    return Promise.resolve(done)
  }
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      waiters.delete(requestId)
      resolve('timeout')
    }, timeoutMs)
    waiters.set(requestId, (r) => {
      clearTimeout(timer)
      resolve(r)
    })
  })
}

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  return typeof err === 'string' ? err : JSON.stringify(err)
}

/** `namespace/name` (or `name` when cluster-scoped). */
export function objectName<R>(d: ResourceDescriptor<R>, row: R): string {
  const ref = d.ref(row)
  return ref.namespace ? `${ref.namespace}/${ref.name}` : ref.name
}

/** `pod default/api-1` for one row, `3 pods` for several. */
export function summarize<R>(d: ResourceDescriptor<R>, rows: readonly R[]): string {
  const [only] = rows
  if (rows.length === 1 && only !== undefined) {
    return `${d.kind.toLowerCase()} ${objectName(d, only)}`
  }
  return `${String(rows.length)} ${d.label.toLowerCase()}`
}

export interface RunOptions {
  /** List pane the rows were selected in: its selection is cleared on success. */
  paneId?: PaneId
  input?: ActionInput
  store?: StoreApi<StoreState>
}

export interface ActionOutcome<R> {
  ok: R[]
  failed: { row: R; error: string }[]
  timedOut: R[]
}

/** Toast test hook for the live E2E suite; the outcome is sonner's `data-type`. */
export const ACTION_TOAST_TEST_ID = 'action-toast'

let toastSeq = 0

/** Send `action` for `rows` (already confirmed by the operator) and report the outcome. */
export async function runAction<R>(
  descriptor: ResourceDescriptor<R>,
  action: ActionDescriptor<R>,
  rows: R[],
  opts: RunOptions = {},
): Promise<ActionOutcome<R>> {
  const store = opts.store ?? useStore
  const input = opts.input ?? {}
  const steps = action.plan(rows, input)
  const outcome: ActionOutcome<R> = { ok: [], failed: [], timedOut: [] }
  if (steps.length === 0) return outcome

  const target =
    summarize(descriptor, rows) +
    (action.id === 'scale' && input.replicas !== undefined ? ` to ${String(input.replicas)}` : '')
  const label = `${action.label} ${target}`
  toastSeq += 1
  const toastId = `action-${String(toastSeq)}`
  const base = { id: toastId, testId: ACTION_TOAST_TEST_ID }
  toast.loading(`${action.doing} ${target}…`, base)

  const runStep = async (step: ActionStep<R>): Promise<StepResult> => {
    let requestId: number
    try {
      requestId = await store.getState().request(step.cmd)
    } catch (err) {
      return { status: 'failed', error: errorText(err) }
    }
    store.setState((s) => ({
      pending: {
        ...s.pending,
        [requestId]: {
          label,
          ...(opts.paneId !== undefined ? { paneId: opts.paneId } : {}),
          sentAt: Date.now(),
        },
      },
    }))
    const result = await waitFor(requestId)
    store.setState((s) => {
      const pending = { ...s.pending }
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- keyed by request id
      delete pending[requestId]
      return { pending }
    })
    if (result === 'timeout') return { status: 'timeout', requestId }
    return result.ok
      ? { status: 'ok' }
      : {
          status: 'failed',
          error: result.error ?? 'The backend reported a failure with no message.',
        }
  }

  // Sends start in order (each `request` is issued before the first await resolves).
  const results = await Promise.all(steps.map(runStep))

  results.forEach((r, i) => {
    const stepRows = steps[i]?.rows ?? []
    if (r.status === 'ok') outcome.ok.push(...stepRows)
    else if (r.status === 'failed') {
      for (const row of stepRows) outcome.failed.push({ row, error: r.error })
    } else outcome.timedOut.push(...stepRows)
  })

  const keyOf = (row: R) => descriptor.rowKey(row)
  const state = store.getState()
  if (outcome.failed.length === 0 && outcome.timedOut.length === 0) {
    toast.success(`✓ ${action.done} ${target}`, {
      ...base,
      description:
        rows.length > 1 ? rows.map((r) => objectName(descriptor, r)).join(', ') : undefined,
    })
  } else {
    toast.error(failureTitle(action, target, rows.length, outcome), {
      ...base,
      description: failureDetail(descriptor, rows.length, steps.length, outcome, results),
      duration: Infinity,
      closeButton: true,
    })
  }

  if (outcome.ok.length > 0) {
    if (opts.paneId !== undefined) {
      const keep = [...outcome.failed.map((f) => f.row), ...outcome.timedOut].map(keyOf)
      state.setSelection(opts.paneId, keep)
    }
    state.refreshSection(descriptor.section)
  }

  // A result that arrives after the timeout is still reported (and refreshes on success).
  results.forEach((r, i) => {
    if (r.status !== 'timeout') return
    const stepRows = steps[i]?.rows ?? []
    waiters.set(r.requestId, (late) => {
      const what = stepRows.map((row) => objectName(descriptor, row)).join(', ')
      if (late.ok) {
        toast.success(`✓ ${action.done} ${what} (late result)`, { testId: ACTION_TOAST_TEST_ID })
        store.getState().refreshSection(descriptor.section)
      } else {
        toast.error(`✖ ${action.label} failed: ${what} (late result)`, {
          testId: ACTION_TOAST_TEST_ID,
          description: late.error ?? undefined,
          duration: Infinity,
          closeButton: true,
        })
      }
    })
  })

  return outcome
}

function failureTitle<R>(
  action: ActionDescriptor<R>,
  target: string,
  total: number,
  o: ActionOutcome<R>,
): string {
  if (o.failed.length === 0) return `✖ No response from backend: ${action.label} ${target}`
  if (total === 1 || o.ok.length + o.timedOut.length === 0) {
    return `✖ ${action.label} failed: ${target}`
  }
  return `✖ ${action.label}: ${String(o.failed.length)} of ${String(total)} failed`
}

function failureDetail<R>(
  d: ResourceDescriptor<R>,
  total: number,
  stepCount: number,
  o: ActionOutcome<R>,
  results: StepResult[],
): string {
  const lines: string[] = []
  const seconds = Math.round(timeoutMs / 1000)
  if (stepCount === 1 || total === 1) {
    // One command: its error applies to every row, so show it once, verbatim.
    const r = results[0]
    if (r?.status === 'failed') lines.push(r.error)
    if (r?.status === 'timeout') {
      lines.push(
        `No result after ${String(seconds)} s. It may still complete; you will be told if it does.`,
      )
    }
    return lines.join('\n')
  }
  for (const f of o.failed) lines.push(`${objectName(d, f.row)}: ${f.error}`)
  for (const row of o.timedOut) {
    lines.push(`${objectName(d, row)}: no response from backend after ${String(seconds)} s`)
  }
  if (o.ok.length > 0) lines.push(`${String(o.ok.length)} succeeded.`)
  return lines.join('\n')
}

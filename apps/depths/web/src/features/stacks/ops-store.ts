/**
 * Previews, confirmations and running operations (US2, shared by US3), fed by the shared socket.
 *
 * - `requestPreview(req)` sends `PreviewOp` or `PreviewSave`. The reply is a `Preview` the dialog
 *   shows verbatim, or a refusal (`Busy`, `Denied`, `Invalid`, and for a save `ValidationFailed`
 *   or `Conflict`). Only one preview flow is open at a time; a newer request (say, a destructive
 *   option switched on) makes every reply to the older one stale.
 * - `confirm()` sends `Confirm{preview_id}`. `OpStarted` closes the flow and opens an operation
 *   for that stack; `PreviewVoid` or `Busy` keeps the flow open with the refusal.
 * - `OpOutput` and `OpFinished` are matched by `operation_id`, so output keeps streaming whether
 *   or not the reply carries `reply_to`. `cancelOp(stack)` sends `Cancel{operation_id}`.
 *
 * Nothing here changes a stack's status: that only moves when the server's `StacksChanged` says so
 * (FR-009).
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type {
  Actor,
  DenyReason,
  EnvText,
  OperationKind,
  OpOutcome,
  Preview,
  PreviewVoidReason,
  RequestId,
  StackDraft,
  StackOp,
  ValidationError,
} from '@/protocol/generated'

export type PreviewRequest =
  | { kind: 'op'; stack: string; op: StackOp }
  | { kind: 'save'; draft: StackDraft; deploy: boolean }

export type Refusal =
  | { kind: 'busy'; stack: string; operationId: string; actor: Actor; running: OperationKind | null }
  | { kind: 'denied'; reason: DenyReason; detail?: string }
  | { kind: 'invalid'; reason: string }
  | { kind: 'void'; reason: PreviewVoidReason }
  | { kind: 'validation'; errors: ValidationError[] }
  | { kind: 'conflict'; currentText: string; currentEnv: EnvText | null; currentHash: string }
  | { kind: 'lost' }

export type FlowPhase = 'previewing' | 'ready' | 'confirming' | 'refused'

export interface Flow {
  request: PreviewRequest
  phase: FlowPhase
  /** The last preview the server built for this flow; kept while a newer one is on its way. */
  preview: Preview | null
  refusal: Refusal | null
}

export interface OpRun {
  operationId: string | null
  stack: string
  kind: OperationKind
  /** Every `OpOutput` chunk, concatenated as received. */
  text: string
  outcome: OpOutcome | null
  cancelling: boolean
  /** The socket dropped while it ran: output after that point was not seen. */
  interrupted: boolean
}

export interface OpsState {
  flow: Flow | null
  /** The latest operation per stack (running or finished). */
  ops: Record<string, OpRun>
  requestPreview: (req: PreviewRequest) => void
  /** Re-issues the open flow's request (after a void or busy refusal). */
  retry: () => void
  /** `acknowledge` sends `acknowledge_findings` (051 FR-018: an admin deploying escape findings). */
  confirm: (acknowledge?: boolean) => void
  dismiss: () => void
  cancelOp: (stack: string) => void
  clearOp: (stack: string) => void
}

export function requestStack(req: PreviewRequest): string {
  return req.kind === 'op' ? req.stack : req.draft.stack
}

export function requestKind(req: PreviewRequest): OperationKind {
  if (req.kind === 'op') return req.op.kind
  return req.draft.base_hash === undefined ? 'create' : 'save'
}

type Pending =
  | { kind: 'preview'; token: number }
  | { kind: 'confirm'; token: number; stack: string; opKind: OperationKind }
  | { kind: 'cancel'; stack: string }

const pending = new Map<RequestId, Pending>()
/** Which stack each running operation belongs to. */
const opStacks = new Map<string, string>()
/** Bumped by every new preview request: replies carrying an older token are ignored. */
let token = 0

export const useOps = create<OpsState>()((set, get) => {
  const send = (req: PreviewRequest): void => {
    const t = ++token
    const id = sendCmd(
      req.kind === 'op'
        ? { type: 'PreviewOp', data: { stack: req.stack, op: req.op } }
        : { type: 'PreviewSave', data: { draft: req.draft, deploy: req.deploy } },
    )
    if (id === null) {
      set((s) => ({ flow: { request: req, phase: 'refused', preview: s.flow?.preview ?? null, refusal: { kind: 'lost' } } }))
      return
    }
    pending.set(id, { kind: 'preview', token: t })
  }

  return {
    flow: null,
    ops: {},

    requestPreview: (req) => {
      set((s) => ({
        // A re-preview of the same flow keeps the old preview on screen until the new one lands.
        flow: { request: req, phase: 'previewing', preview: s.flow ? s.flow.preview : null, refusal: null },
      }))
      send(req)
    },

    retry: () => {
      const flow = get().flow
      if (!flow) return
      set({ flow: { ...flow, phase: 'previewing', preview: null, refusal: null } })
      send(flow.request)
    },

    confirm: (acknowledge = false) => {
      const flow = get().flow
      if (flow?.phase !== 'ready' || !flow.preview) return
      const preview_id = flow.preview.preview_id
      const id = sendCmd({
        type: 'Confirm',
        data: acknowledge ? { preview_id, acknowledge_findings: true } : { preview_id },
      })
      if (id === null) {
        set({ flow: { ...flow, phase: 'refused', refusal: { kind: 'lost' } } })
        return
      }
      pending.set(id, { kind: 'confirm', token, stack: flow.preview.stack, opKind: flow.preview.kind })
      set({ flow: { ...flow, phase: 'confirming' } })
    },

    dismiss: () => {
      token++
      set({ flow: null })
    },

    cancelOp: (stack) => {
      const run = get().ops[stack]
      if (!run?.operationId || run.outcome !== null) return
      const id = sendCmd({ type: 'Cancel', data: { operation_id: run.operationId } })
      if (id === null) return
      pending.set(id, { kind: 'cancel', stack })
      set((s) => ({ ops: { ...s.ops, [stack]: { ...run, cancelling: true } } }))
    },

    clearOp: (stack) => {
      set((s) => {
        const run = s.ops[stack]
        if (!run || (run.outcome === null && !run.interrupted)) return s
        return { ops: Object.fromEntries(Object.entries(s.ops).filter(([k]) => k !== stack)) }
      })
    },
  }
})

function refuse(t: number, refusal: Refusal): void {
  const { flow } = useOps.getState()
  if (!flow || t !== token) return
  useOps.setState({ flow: { ...flow, phase: 'refused', refusal } })
}

function updateRun(operationId: string, f: (r: OpRun) => OpRun): void {
  const stack = opStacks.get(operationId)
  if (stack === undefined) return
  const run = useOps.getState().ops[stack]
  if (run?.operationId !== operationId) return
  useOps.setState((s) => ({ ops: { ...s.ops, [stack]: f(run) } }))
}

onServerEvent(({ reply_to, event }) => {
  const asked = reply_to === undefined ? undefined : pending.get(reply_to)

  switch (event.type) {
    case 'Preview': {
      if (asked?.kind !== 'preview' || reply_to === undefined) return
      pending.delete(reply_to)
      const { flow } = useOps.getState()
      if (!flow || asked.token !== token) return
      useOps.setState({ flow: { ...flow, phase: 'ready', preview: event.data, refusal: null } })
      return
    }
    case 'OpStarted': {
      if (asked?.kind !== 'confirm' || reply_to === undefined) return
      pending.delete(reply_to)
      const operationId = event.data.operation_id
      opStacks.set(operationId, asked.stack)
      const run: OpRun = {
        operationId,
        stack: asked.stack,
        kind: asked.opKind,
        text: '',
        outcome: null,
        cancelling: false,
        interrupted: false,
      }
      useOps.setState((s) => ({
        ops: { ...s.ops, [asked.stack]: run },
        flow: asked.token === token ? null : s.flow,
      }))
      return
    }
    case 'OpOutput':
      updateRun(event.data.operation_id, (r) => ({ ...r, text: r.text + event.data.chunk }))
      return
    case 'OpFinished': {
      const { operation_id, outcome } = event.data
      updateRun(operation_id, (r) => ({ ...r, outcome, cancelling: false }))
      opStacks.delete(operation_id)
      if (reply_to !== undefined) pending.delete(reply_to)
      return
    }
    case 'Busy': {
      if (!asked || asked.kind === 'cancel' || reply_to === undefined) return
      pending.delete(reply_to)
      const running = useOps.getState().ops[event.data.stack]
      refuse(asked.token, {
        kind: 'busy',
        stack: event.data.stack,
        operationId: event.data.operation_id,
        actor: event.data.actor,
        running: running?.operationId === event.data.operation_id ? running.kind : null,
      })
      return
    }
    case 'PreviewVoid':
      if (asked?.kind !== 'confirm' || reply_to === undefined) return
      pending.delete(reply_to)
      refuse(asked.token, { kind: 'void', reason: event.data.reason })
      return
    case 'Denied':
    case 'Invalid':
    case 'ValidationFailed':
    case 'Conflict': {
      if (!asked || reply_to === undefined) return
      pending.delete(reply_to)
      if (asked.kind === 'cancel') {
        const run = useOps.getState().ops[asked.stack]
        if (run) useOps.setState((s) => ({ ops: { ...s.ops, [asked.stack]: { ...run, cancelling: false } } }))
        return
      }
      refuse(
        asked.token,
        event.type === 'Denied'
          ? event.data.detail
            ? { kind: 'denied', reason: event.data.reason, detail: event.data.detail }
            : { kind: 'denied', reason: event.data.reason }
          : event.type === 'Invalid'
            ? { kind: 'invalid', reason: event.data.reason }
            : event.type === 'ValidationFailed'
              ? { kind: 'validation', errors: event.data.errors }
              : {
                  kind: 'conflict',
                  currentText: event.data.current_text,
                  currentEnv: event.data.current_env,
                  currentHash: event.data.current_hash,
                },
      )
      return
    }
    default:
      return
  }
})

onReconnected(() => {
  // Replies to anything in flight went with the old socket.
  const inFlight = [...pending.values()]
  pending.clear()
  const { flow, ops } = useOps.getState()
  if (flow?.phase === 'previewing') useOps.getState().requestPreview(flow.request)
  // A confirm whose reply was lost may or may not have run: say so rather than guess.
  else if (flow?.phase === 'confirming') useOps.setState({ flow: { ...flow, phase: 'refused', refusal: { kind: 'lost' } } })
  const next: Record<string, OpRun> = {}
  for (const [stack, run] of Object.entries(ops)) {
    next[stack] = run.outcome === null ? { ...run, interrupted: true, cancelling: false } : run
  }
  if (inFlight.length > 0 || Object.keys(next).length > 0) useOps.setState({ ops: next })
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') resetOpsStore()
})

/** Test hook, and sign-out: forget everything. */
export function resetOpsStore(): void {
  pending.clear()
  opStacks.clear()
  token++
  useOps.setState({ flow: null, ops: {} })
}

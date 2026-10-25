/**
 * The preview → confirm flow for changes to files (spec 053, US2–US5), over the shared socket.
 *
 * - `request(stack, op, after?)` sends `PreviewFileOp`. The reply is a `FilePreview` (summary,
 *   diff, entry count, upload targets, running services) the dialog shows verbatim, or a refusal.
 *   One flow is open at a time; a newer request makes replies to the older one stale.
 * - `confirm(choice?)` sends `Confirm{preview_id}`. `FileOpDone` ends the flow and calls the
 *   request's `after` with the result and the choice (say, "save and restart app"), so the caller
 *   chains what comes next. `FileRefused`, `PreviewVoid` and `Denied` keep the flow open with the
 *   refusal; a save's `changed_on_disk` is shown by the editor, not the dialog.
 * - Nothing is written by the preview; nothing here writes the file list. Callers re-list.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type { Event, FileOp, FilePreview, PreviewVoidReason, RequestId } from '@/protocol/generated'

import type { FilesRefusal } from './store'

export type FileOpDone = Extract<Event, { type: 'FileOpDone' }>['data']

/** What the person chose when confirming: plain, or followed by a lifecycle op on a service. */
export type ConfirmChoice = { then: 'none' } | { then: 'restart'; service: string }

export type FileOpRefusal = FilesRefusal | { kind: 'void'; reason: PreviewVoidReason } | { kind: 'lost' }

export type FileFlowPhase = 'previewing' | 'ready' | 'confirming' | 'refused'

export interface FileFlow {
  stack: string
  op: FileOp
  phase: FileFlowPhase
  preview: FilePreview | null
  refusal: FileOpRefusal | null
  /** Called once with the server's `FileOpDone`. */
  after?: (done: FileOpDone, choice: ConfirmChoice) => void
}

export interface FileOpsState {
  flow: FileFlow | null
  /** The last change that landed, for the `aria-live` status line. */
  lastDone: { summary: string; at: number } | null
  request: (stack: string, op: FileOp, after?: FileFlow['after']) => void
  /** Previews the open flow's op again (after a void or a lost reply). */
  retry: () => void
  confirm: (choice?: ConfirmChoice) => void
  dismiss: () => void
}

type Pending = { kind: 'preview'; token: number } | { kind: 'confirm'; token: number; choice: ConfirmChoice }

const pending = new Map<RequestId, Pending>()
let token = 0
let doneSeq = 0

export const useFileOps = create<FileOpsState>()((set, get) => {
  const send = (stack: string, op: FileOp): void => {
    const t = ++token
    const id = sendCmd({ type: 'PreviewFileOp', data: { stack, op } })
    if (id === null) {
      set((s) => (s.flow ? { flow: { ...s.flow, phase: 'refused', refusal: { kind: 'lost' } } } : s))
      return
    }
    pending.set(id, { kind: 'preview', token: t })
  }

  return {
    flow: null,
    lastDone: null,

    request: (stack, op, after) => {
      set({ flow: { stack, op, phase: 'previewing', preview: null, refusal: null, ...(after ? { after } : {}) } })
      send(stack, op)
    },

    retry: () => {
      const flow = get().flow
      if (!flow) return
      set({ flow: { ...flow, phase: 'previewing', preview: null, refusal: null } })
      send(flow.stack, flow.op)
    },

    confirm: (choice = { then: 'none' }) => {
      const flow = get().flow
      if (flow?.phase !== 'ready' || !flow.preview) return
      const id = sendCmd({ type: 'Confirm', data: { preview_id: flow.preview.preview_id } })
      if (id === null) {
        set({ flow: { ...flow, phase: 'refused', refusal: { kind: 'lost' } } })
        return
      }
      pending.set(id, { kind: 'confirm', token, choice })
      set({ flow: { ...flow, phase: 'confirming' } })
    },

    dismiss: () => {
      token++
      set({ flow: null })
    },
  }
})

function refuse(t: number, refusal: FileOpRefusal): void {
  const { flow } = useFileOps.getState()
  if (!flow || t !== token) return
  useFileOps.setState({ flow: { ...flow, phase: 'refused', refusal } })
}

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined) return
  const asked = pending.get(reply_to)
  if (!asked) return
  pending.delete(reply_to)
  switch (event.type) {
    case 'FilePreview': {
      const { flow } = useFileOps.getState()
      if (asked.kind !== 'preview' || !flow || asked.token !== token) return
      useFileOps.setState({ flow: { ...flow, phase: 'ready', preview: event.data, refusal: null } })
      return
    }
    case 'FileOpDone': {
      const { flow } = useFileOps.getState()
      if (asked.kind !== 'confirm' || !flow || asked.token !== token) return
      useFileOps.setState({ flow: null, lastDone: { summary: flow.preview?.summary ?? '', at: ++doneSeq } })
      flow.after?.(event.data, asked.choice)
      return
    }
    case 'FileRefused':
      refuse(asked.token, event.data)
      return
    case 'Denied':
      refuse(asked.token, { kind: 'denied', reason: event.data.reason })
      return
    case 'PreviewVoid':
      refuse(asked.token, { kind: 'void', reason: event.data.reason })
      return
    default:
      // Not a reply this flow understands: keep waiting for one.
      pending.set(reply_to, asked)
  }
})

onReconnected(() => {
  pending.clear()
  const { flow } = useFileOps.getState()
  if (flow?.phase === 'previewing') useFileOps.getState().retry()
  // A confirm whose reply was lost may or may not have landed: say so rather than guess.
  else if (flow?.phase === 'confirming') useFileOps.setState({ flow: { ...flow, phase: 'refused', refusal: { kind: 'lost' } } })
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') resetFileOpsStore()
})

/** Test hook, and sign-out. */
export function resetFileOpsStore(): void {
  pending.clear()
  token++
  useFileOps.setState({ flow: null, lastDone: null })
}

/** A save refused because the file changed on disk: the editor offers reload or compare. */
export function isSaveConflict(flow: FileFlow | null): boolean {
  return flow?.op.kind === 'save' && flow.phase === 'refused' && flow.refusal?.kind === 'changed_on_disk'
}

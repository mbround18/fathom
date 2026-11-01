/**
 * Approving or denying a held run (spec 064 US1, US2): the dialog's state, apart from the runs
 * store's cancel so a refusal here shows in the dialog rather than at the top of the page.
 *
 * `start(kind, run)` sends `PreviewRunApprove` or `PreviewRunDeny` and opens the dialog while the
 * preview comes back. `confirm()` answers it: a deny whose Reason changed since the preview is
 * previewed again with the reason, then confirmed as soon as that preview arrives. Both previews ask
 * for step-up; when the session has not just verified, the passkey dialog runs first, and closing it
 * sends nothing, so the run keeps waiting.
 *
 * The server answers `Confirmed`, then the run as it now is (`Run`, which the runs store shows) or a
 * refusal (`OpError`, `Denied`), kept here and shown in the dialog with the server's words. After a
 * refusal the run is read again, since it may have moved on (answered, or expired on the spot).
 */
import { create } from 'zustand'

import { onServerEvent, sendCmd } from '@/app/connection'
import { OFFLINE, refusalOf, type Refusal } from '@/app/load'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type { Cmd, Preview, RequestId } from '@/protocol/wire'

/** The longest reason the server keeps. */
export const MAX_REASON = 500

export type DecisionKind = 'approve' | 'deny'

export interface DecisionState {
  /** What the open dialog does; `null` when it is closed. */
  kind: DecisionKind | null
  run: string | null
  preview: Preview | null
  /** The reason `preview` was made with (a deny's). */
  previewReason: string
  /** The Reason field, for a deny. */
  reason: string
  refusal: Refusal | null
  /** Waiting on the server: the preview, or the answer to Confirm. */
  busy: boolean
  start: (kind: DecisionKind, run: string) => void
  setReason: (reason: string) => void
  confirm: () => Promise<void>
  close: () => void
}

const mine = new Set<RequestId>()
/** Confirm the next preview as soon as it arrives: a deny re-previewed with its reason. */
let confirmNext = false

/** Called after an answer lands, so the waiting count follows (spec 064 US4). */
let answered: () => void = () => undefined
export function onAnswered(f: () => void): void {
  answered = f
}

function send(cmd: Cmd): boolean {
  const id = sendCmd(cmd)
  if (id === null) {
    useDecision.setState({ refusal: OFFLINE, busy: false })
    return false
  }
  mine.add(id)
  return true
}

function previewCmd(kind: DecisionKind, run: string, reason: string): Cmd {
  return kind === 'approve'
    ? { type: 'PreviewRunApprove', data: { run } }
    : { type: 'PreviewRunDeny', data: { run, reason: reason === '' ? null : reason } }
}

const CLOSED = { kind: null, run: null, preview: null, previewReason: '', reason: '', refusal: null, busy: false }

export const useDecision = create<DecisionState>()((set, get) => ({
  ...CLOSED,

  start: (kind, run) => {
    confirmNext = false
    set({ ...CLOSED, kind, run, busy: true })
    send(previewCmd(kind, run, ''))
  },
  setReason: (reason) => {
    set({ reason: reason.slice(0, MAX_REASON) })
  },
  confirm: async () => {
    const { kind, run, preview, previewReason, reason } = get()
    if (!kind || !run || !preview) return
    const wanted = kind === 'deny' ? reason.trim() : ''
    if (wanted !== previewReason) {
      // The preview the person read had no reason (or another one): preview what they will send.
      confirmNext = true
      set({ busy: true, refusal: null, previewReason: wanted })
      send(previewCmd(kind, run, wanted))
      return
    }
    set({ busy: true, refusal: null })
    const verified = useSession.getState().me?.verification.verified ?? false
    if (preview.step_up && !verified) {
      const assertion = await usePasskeys.getState().requestStepUp(preview.summary)
      // Closed: nothing is recorded and the run keeps waiting.
      if (!assertion) {
        set({ busy: false })
        return
      }
      if (get().preview !== preview) return
      send({ type: 'Confirm', data: { preview_id: preview.preview_id, step_up: assertion } })
    } else {
      send({ type: 'Confirm', data: { preview_id: preview.preview_id } })
    }
  },
  close: () => {
    confirmNext = false
    mine.clear()
    set(CLOSED)
  },
}))

onServerEvent(({ reply_to, event }) => {
  if (reply_to === undefined || !mine.has(reply_to)) return
  const s = useDecision.getState()
  switch (event.type) {
    case 'Preview':
      useDecision.setState({ preview: event.data, busy: false })
      if (confirmNext) {
        confirmNext = false
        void s.confirm()
      }
      break
    case 'Confirmed':
      // The answer follows.
      break
    case 'Run':
      if (event.data.run.id === s.run) {
        mine.delete(reply_to)
        s.close()
        answered()
      }
      break
    default: {
      const refusal = refusalOf(event)
      if (!refusal) break
      mine.delete(reply_to)
      confirmNext = false
      useDecision.setState({ refusal, preview: null, busy: false })
      // The run may have moved on (answered by someone else, or expired on the spot): show it as it is.
      if (s.run) sendCmd({ type: 'GetRun', data: { run: s.run } })
      answered()
    }
  }
})

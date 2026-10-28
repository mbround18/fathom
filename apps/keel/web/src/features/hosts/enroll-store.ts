/**
 * The Enroll dialog's flow (spec 060 US1, contracts/web-ui.md).
 *
 * Step 1 is the form; "Continue" sends `PreviewHostEnroll` and shows the server's preview in the
 * same dialog; Confirm asks for a passkey when the preview says so and the session has not just
 * verified, then sends `Confirm`. The answer is `Confirmed` and `HostEnrollment`, or a refusal
 * shown verbatim.
 *
 * Step 2 shows what `HostEnrollment` carries: the token, the rendered snippet and the
 * instructions. A Roll (`PreviewEnrollmentRoll`, from Settings → Tokens) ends in the same step.
 * The token lives in this store's memory only and is dropped when the dialog closes; it is never
 * written to storage or logged.
 */
import { create } from 'zustand'

import { onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import { usePasskeys } from '@/features/passkeys/store'
import type { Refusal } from '@/features/teams/store'
import type { Cmd, Event, PasskeyAssertion, Preview, RequestId } from '@/protocol/wire'

/** What `HostEnrollment` carries. */
export type Minted = Extract<Event, { type: 'HostEnrollment' }>['data']

export type EnrollPhase = 'closed' | 'form' | 'asking' | 'preview' | 'stepping' | 'confirming' | 'result' | 'refused'

/** The `HostName` rule (keel-protocol): 1 to 253 of A-Z a-z 0-9 . _ -, starting alphanumeric. */
export const HOST_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$/

export const MAX_PROFILES = 32

export interface EnrollState {
  phase: EnrollPhase
  /** Enroll or Roll: the dialog's title. */
  mode: 'enroll' | 'roll'
  /** The workspace the form starts on. */
  initialWorkspace: string | null
  preview: Preview | null
  refusal: Refusal | null
  minted: Minted | null
  /** Bumped on every token minted, so lists of enrolment tokens re-read. */
  mintedCount: number
  open: (workspace: string | null) => void
  submit: (workspace: string, inventory_name: string, profiles: string[]) => void
  roll: (enrollment: string) => void
  confirm: () => Promise<void>
  /** Back to the form, from a preview or a refusal. */
  back: () => void
  close: () => void
}

const OFFLINE: Refusal = {
  kind: 'error',
  code: 'offline',
  message: 'Keel is not connected. Wait for it to reconnect, then try again.',
}
const ENROL_FIRST: Refusal = {
  kind: 'error',
  code: 'passkey_required',
  message: 'This change needs a passkey. Register one, then confirm again.',
}

const handlers = new Map<RequestId, (event: Event) => void>()
let generation = 0

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined) handlers.get(reply_to)?.(event)
})

function refusalOf(event: Event): Refusal | null {
  if (event.type === 'Denied') return { kind: 'denied', action: event.data.action, reason: event.data.reason }
  if (event.type === 'OpError') return { kind: 'error', code: event.data.code, message: event.data.message }
  if (event.type === 'PasskeyEnrolmentRequired') return ENROL_FIRST
  return null
}

export const useEnroll = create<EnrollState>()((set, get) => {
  /** Sends `cmd` for the current flow; replies after the flow moved on are dropped. */
  const send = (cmd: Cmd, handle: (event: Event) => void): void => {
    const gen = generation
    const id = sendCmd(cmd)
    if (id === null) {
      set({ phase: 'refused', refusal: OFFLINE })
      return
    }
    handlers.set(id, (event) => {
      if (gen === generation) handle(event)
    })
  }

  const ask = (cmd: Cmd): void => {
    generation++
    set({ phase: 'asking', preview: null, refusal: null })
    send(cmd, (event) => {
      if (event.type === 'Preview') set({ phase: 'preview', preview: event.data })
      else {
        const refusal = refusalOf(event)
        if (refusal) set({ phase: 'refused', refusal })
      }
    })
  }

  return {
    phase: 'closed',
    mode: 'enroll',
    initialWorkspace: null,
    preview: null,
    refusal: null,
    minted: null,
    mintedCount: 0,

    open: (workspace) => {
      generation++
      set({ phase: 'form', mode: 'enroll', initialWorkspace: workspace, preview: null, refusal: null, minted: null })
    },

    submit: (workspace, inventory_name, profiles) => {
      set({ mode: 'enroll', initialWorkspace: workspace })
      ask({ type: 'PreviewHostEnroll', data: { workspace, inventory_name, profiles } })
    },

    roll: (enrollment) => {
      set({ mode: 'roll', minted: null })
      ask({ type: 'PreviewEnrollmentRoll', data: { enrollment } })
    },

    confirm: async () => {
      const { preview, phase } = get()
      if (!preview || phase !== 'preview') return
      const gen = generation
      const me = useSession.getState().me
      let stepUp: PasskeyAssertion | null = null
      if (preview.step_up && !me?.verification.verified && me?.verification.enrolled) {
        set({ phase: 'stepping' })
        stepUp = await usePasskeys.getState().requestStepUp(preview.summary)
        if (gen !== generation) return
        if (!stepUp) {
          set({ phase: 'preview' })
          return
        }
      }
      set({ phase: 'confirming', refusal: null })
      send(
        {
          type: 'Confirm',
          data: stepUp ? { preview_id: preview.preview_id, step_up: stepUp } : { preview_id: preview.preview_id },
        },
        (event) => {
          if (event.type === 'Confirmed') return
          if (event.type === 'HostEnrollment') {
            set((s) => ({ phase: 'result', minted: event.data, preview: null, mintedCount: s.mintedCount + 1 }))
            return
          }
          const refusal = refusalOf(event)
          if (!refusal) return
          // A spent preview cannot be confirmed again: the person starts over.
          set({ phase: 'refused', refusal, preview: null })
          if (refusal === ENROL_FIRST) usePasskeys.getState().openEnrol(true)
        },
      )
    },

    back: () => {
      generation++
      const { mode } = get()
      if (mode === 'roll') get().close()
      else set({ phase: 'form', preview: null, refusal: null })
    },

    close: () => {
      generation++
      handlers.clear()
      set({ phase: 'closed', preview: null, refusal: null, minted: null })
    },
  }
})

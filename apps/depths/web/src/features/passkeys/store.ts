/**
 * Passkeys and verification (spec 054 T065), over the shared socket. Adapted from Keel's
 * `features/passkeys/store.ts`.
 *
 * - `verification` follows every `Verification` event: the reply to `GetVerification` (sent each
 *   time the socket opens), the one the server pushes after sign-in when an admin still has to
 *   verify, and the ones that answer `PasskeyFinish` and `RecoverPasskeys`.
 * - One dialog at a time: `enrol` (register a passkey), `verify` (confirm it's you after sign-in)
 *   or `step_up` (a fresh check one action asked for). Each runs one ceremony: `PasskeyBegin` →
 *   `PasskeyChallenge` → the authenticator → `PasskeyFinish` → `PasskeyRegistered` or
 *   `Verification`. A `step_up` that answers `StepUpRequired{request}` uses that request; asked
 *   again after a refusal it begins a fresh `verify` ceremony.
 * - `requestStepUp(summary, request)` resolves with an assertion for a command that carries one
 *   (`OpenShell.assertion`), or `null` when the dialog is closed. `requestFreshCheck(summary,
 *   request)` finishes the ceremony itself (`PasskeyFinish`), for commands that carry none
 *   (`PutUser`, `PreviewFileOp`): it resolves `true` once the session is fresh.
 * - A `PasskeyEnrolmentRequired` opens the enrol dialog, unless the admin gate already says so.
 * - `list` is the person's passkeys (`ListPasskeys`), refreshed after each change.
 */
import { create } from 'zustand'

import { onReconnected, onServerEvent, sendCmd } from '@/app/connection'
import { useSession } from '@/app/session-store'
import type {
  Cmd,
  Event,
  PasskeyAssertion,
  PasskeyInfo,
  PasskeyRequest,
  Preview,
  RequestId,
  Verification,
} from '@/protocol/generated'

import { createCredential, getAssertion, PasskeyFailure } from './api'
import { NO_AUTHENTICATOR, OFFLINE, REFUSED_MESSAGES, STEP_UP_AGAIN } from './messages'

export type PasskeyDialog =
  | { kind: 'enrol'; required: boolean }
  | { kind: 'verify' }
  | { kind: 'step_up'; summary: string; request: PasskeyRequest | null; finish: boolean }

export type PasskeyPhase = 'idle' | 'server' | 'authenticator' | 'refused' | 'done'

export interface PasskeyState {
  verification: Verification | null
  dialog: PasskeyDialog | null
  phase: PasskeyPhase
  error: string | null
  /** The sign-in prompt was shown (and maybe cancelled) for this session: don't push it again. */
  prompted: boolean
  list: PasskeyInfo[] | null
  refreshVerification: () => void
  openEnrol: (required?: boolean) => void
  openVerify: () => void
  requestStepUp: (summary: string, request: PasskeyRequest | null) => Promise<PasskeyAssertion | null>
  requestFreshCheck: (summary: string, request: PasskeyRequest | null) => Promise<boolean>
  close: () => void
  enrol: (name: string) => Promise<void>
  verify: () => Promise<void>
  loadList: () => void
  /** `RecoverPasskeys`: `null` when accepted, the message otherwise. */
  recover: (code: string) => Promise<string | null>
  /** `PreviewPasskeyRemove`: the preview `Confirm` acts on, or the message. */
  previewRemove: (id: string) => Promise<{ ok: true; preview: Preview } | { ok: false; message: string }>
  /**
   * `Confirm` of a removal preview (with its assertion, when it asked for one): `null` when
   * removed, otherwise the message, and the fresh request when the server wants a new check.
   */
  confirmRemove: (preview: Preview, assertion?: PasskeyAssertion) => Promise<{ message: string; stepUp?: PasskeyRequest } | null>
}

/** Thrown inside a ceremony to end it with a message. */
class Refused extends Error {}

const waiting = new Map<RequestId, (event: Event) => void>()

/** Send `cmd` and wait for the event that answers it. */
function ask(cmd: Cmd): Promise<Event> {
  const id = sendCmd(cmd)
  if (id === null) return Promise.reject(new Refused(OFFLINE))
  return new Promise((resolve) => waiting.set(id, resolve))
}

/** The message for an event that refused a passkey command (or one it did not expect). */
export function refusalMessage(event: Event): string {
  switch (event.type) {
    case 'PasskeyRefused':
      return REFUSED_MESSAGES[event.data.reason]
    case 'PasskeyEnrolmentRequired':
      return 'Register a passkey first.'
    case 'StepUpRequired':
      return 'Confirm it\'s you first, then try again.'
    case 'Denied':
      return event.data.detail ?? 'You are not permitted to do that.'
    case 'Invalid':
      return event.data.reason
    default:
      return `Unexpected answer from the server (${event.type}).`
  }
}

function failure(e: unknown): string {
  if (e instanceof PasskeyFailure) return e.reason === 'unsupported' ? NO_AUTHENTICATOR : REFUSED_MESSAGES[e.reason === 'invalid' ? 'invalid' : 'cancelled']
  return e instanceof Error ? e.message : String(e)
}

/** Resolves the pending `requestStepUp` / `requestFreshCheck`, if any. */
let stepUpResolve: ((a: PasskeyAssertion | null) => void) | null = null
/** Bumped on every open and close, so an attempt from a closed dialog does nothing. */
let generation = 0

export const usePasskeys = create<PasskeyState>()((set, get) => {
  const open = (dialog: PasskeyDialog): void => {
    generation++
    set({ dialog, phase: 'idle', error: null })
  }

  /** Run `body` as the current attempt; failures become the dialog's message. */
  const attempt = async (body: (live: () => boolean) => Promise<void>): Promise<void> => {
    const gen = ++generation
    const live = () => gen === generation
    set({ phase: 'server', error: null })
    try {
      await body(live)
    } catch (e) {
      if (!live()) return
      set({ phase: 'refused', error: failure(e) })
    }
  }

  /** A verify ceremony's request: the one given, or a fresh one from the server. */
  const verifyRequest = async (given: PasskeyRequest | null): Promise<PasskeyRequest> => {
    if (given) return given
    const challenge = await ask({ type: 'PasskeyBegin', data: { purpose: { kind: 'verify' } } })
    if (challenge.type !== 'PasskeyChallenge') throw new Refused(refusalMessage(challenge))
    return challenge.data
  }

  /** Ends the dialog and hands `value` to whoever asked for the step-up. */
  const settle = (value: PasskeyAssertion | null): void => {
    const resolve = stepUpResolve
    stepUpResolve = null
    generation++
    set({ dialog: null, phase: 'idle', error: null })
    resolve?.(value)
  }

  const askStepUp = (summary: string, request: PasskeyRequest | null, finish: boolean): Promise<PasskeyAssertion | null> => {
    stepUpResolve?.(null)
    open({ kind: 'step_up', summary, request, finish })
    return new Promise((resolve) => {
      stepUpResolve = resolve
    })
  }

  return {
    verification: null,
    dialog: null,
    phase: 'idle',
    error: null,
    prompted: false,
    list: null,

    refreshVerification: () => {
      void ask({ type: 'GetVerification' }).catch(() => undefined)
    },

    openEnrol: (required = false) => {
      const d = get().dialog
      // A required enrolment does not replace a ceremony in progress.
      if (d && get().phase !== 'idle' && get().phase !== 'refused') return
      if (d?.kind === 'step_up') settle(null)
      open({ kind: 'enrol', required })
    },

    openVerify: () => {
      set({ prompted: true })
      open({ kind: 'verify' })
    },

    requestStepUp: (summary, request) => askStepUp(summary, request, false),

    requestFreshCheck: async (summary, request) => (await askStepUp(summary, request, true)) !== null,

    close: () => {
      if (stepUpResolve) settle(null)
      else {
        generation++
        set({ dialog: null, phase: 'idle', error: null })
      }
    },

    enrol: (name) =>
      attempt(async (live) => {
        const challenge = await ask({ type: 'PasskeyBegin', data: { purpose: { kind: 'register', name: name.trim() } } })
        if (!live()) return
        if (challenge.type !== 'PasskeyChallenge') throw new Refused(refusalMessage(challenge))
        set({ phase: 'authenticator' })
        const credential = await createCredential(challenge.data.options)
        if (!live()) return
        set({ phase: 'server' })
        const done = await ask({ type: 'PasskeyFinish', data: { ceremony: challenge.data.ceremony, credential } })
        if (!live()) return
        if (done.type !== 'PasskeyRegistered') throw new Refused(refusalMessage(done))
        set({ phase: 'done' })
        get().loadList()
        // Registering verifies the session too (and ends a recovery); the server does not push it.
        get().refreshVerification()
      }),

    verify: () =>
      attempt(async (live) => {
        const d = get().dialog
        const stepUp = d?.kind === 'step_up' ? d : null
        // The first try answers the request the action came with; a retry (that ceremony is
        // spent or expired) begins a fresh one.
        const given = stepUp ? stepUp.request : null
        if (stepUp) set({ dialog: { ...stepUp, request: null } })
        const request = await verifyRequest(given)
        if (!live()) return
        set({ phase: 'authenticator' })
        const credential = await getAssertion(request.options)
        if (!live()) return
        if (stepUp && !stepUp.finish) {
          settle({ ceremony: request.ceremony, credential })
          return
        }
        set({ phase: 'server' })
        const done = await ask({ type: 'PasskeyFinish', data: { ceremony: request.ceremony, credential } })
        if (!live()) return
        if (done.type !== 'Verification') throw new Refused(refusalMessage(done))
        if (stepUp) settle({ ceremony: request.ceremony, credential })
        else set({ phase: 'done' })
      }),

    loadList: () => {
      void ask({ type: 'ListPasskeys' })
        .then((e) => {
          if (e.type === 'Passkeys') set({ list: e.data.items })
        })
        .catch(() => undefined)
    },

    recover: async (code) => {
      try {
        const e = await ask({ type: 'RecoverPasskeys', data: { code: code.trim() } })
        return e.type === 'Verification' ? null : refusalMessage(e)
      } catch (e) {
        return failure(e)
      }
    },

    previewRemove: async (id) => {
      try {
        const e = await ask({ type: 'PreviewPasskeyRemove', data: { id } })
        return e.type === 'Preview' ? { ok: true, preview: e.data } : { ok: false, message: refusalMessage(e) }
      } catch (e) {
        return { ok: false, message: failure(e) }
      }
    },

    confirmRemove: async (preview, assertion) => {
      try {
        const e = await ask({
          type: 'Confirm',
          data: assertion ? { preview_id: preview.preview_id, assertion } : { preview_id: preview.preview_id },
        })
        if (e.type === 'Passkeys') {
          set({ list: e.data.items })
          return null
        }
        if (e.type === 'StepUpRequired') return { message: STEP_UP_AGAIN, stepUp: e.data.request }
        return { message: refusalMessage(e) }
      } catch (e) {
        return { message: failure(e) }
      }
    },
  }
})

onServerEvent(({ reply_to, event }) => {
  if (event.type === 'Verification') usePasskeys.setState({ verification: event.data })
  if (reply_to !== undefined) {
    const resolve = waiting.get(reply_to)
    if (resolve) {
      waiting.delete(reply_to)
      resolve(event)
      return
    }
  }
  // Anyone else's command (or a push) that needs a passkey first. An admin who has none is
  // already told by the gate's banner, so only a plain user gets the dialog this way.
  if (event.type === 'PasskeyEnrolmentRequired' && usePasskeys.getState().verification?.required !== true) {
    usePasskeys.getState().openEnrol(true)
  }
})

onReconnected(() => {
  // Replies to anything in flight went with the old socket.
  for (const resolve of waiting.values()) resolve({ type: 'Invalid', data: { reason: OFFLINE } })
  waiting.clear()
})

useSession.subscribe((s, prev) => {
  if (s.status !== 'ready' && prev.status === 'ready') {
    resetPasskeysStore()
    return
  }
  if (s.connection === 'open' && prev.connection !== 'open') usePasskeys.getState().refreshVerification()
})

/** Test hook, and sign-out: forget everything. */
export function resetPasskeysStore(): void {
  waiting.clear()
  stepUpResolve?.(null)
  stepUpResolve = null
  generation++
  usePasskeys.setState({ verification: null, dialog: null, phase: 'idle', error: null, prompted: false, list: null })
}

/**
 * The passkey dialogs' state (spec 055 T030, the spec 054 ceremony).
 *
 * Two dialogs: enrol (register a passkey) and step-up (prove it is you). Both run one ceremony:
 * `PasskeyBegin` → the server's `PasskeyChallenge` → the browser's authenticator → `PasskeyFinish`
 * → `PasskeyRegistered` or `Verification`. Every reply is matched on `reply_to`; a `PasskeyRefused`
 * or `OpError` reply ends the attempt with a message and the dialog stays open to retry.
 *
 * `requestStepUp` is for actions that carry their own proof (`Confirm.step_up`): it opens the
 * step-up dialog and resolves with the assertion, without `PasskeyFinish`, or `null` when closed.
 *
 * A pushed `PasskeyEnrolmentRequired` opens the enrol dialog as required.
 */
import { create } from 'zustand'

import { onServerEvent, sendCmd } from '@/app/connection'
import {
  assertionJson,
  attestationJson,
  browserAuthenticator,
  creationOptions,
  requestOptions,
  type Authenticator,
} from '@/protocol/webauthn'
import type {
  Event,
  PasskeyAssertion,
  PasskeyCredential,
  PasskeyPurpose,
  PasskeyRefusedReason,
  RequestId,
} from '@/protocol/wire'

export type PasskeyDialog =
  | { kind: 'enrol'; required: boolean }
  | { kind: 'step_up'; mode: 'verify' | 'assert'; summary: string | null }

export type PasskeyPhase = 'idle' | 'server' | 'authenticator' | 'refused' | 'done'

export const REFUSED_MESSAGES: Record<PasskeyRefusedReason, string> = {
  replayed: 'That passkey answer was already used. Try again.',
  wrong_origin: 'The passkey answered for a different site.',
  unknown_credential: 'That passkey is not registered to you.',
  expired: 'The request took too long and expired. Try again.',
  cancelled: 'The passkey prompt was cancelled.',
  recovery_invalid: 'That recovery code is not valid.',
  invalid: 'The passkey answer was not valid. Try again.',
  last_passkey: 'This is your last passkey, so it cannot be removed.',
}

const NO_AUTHENTICATOR = 'This browser cannot use passkeys.'
export const ENROLMENT_REQUIRED = 'Register a passkey first.'
export const OFFLINE = 'Keel is not connected. Wait for it to reconnect, then try again.'

export interface PasskeyState {
  dialog: PasskeyDialog | null
  phase: PasskeyPhase
  error: string | null
  openEnrol: (required?: boolean) => void
  openStepUp: () => void
  /** Ask for a fresh assertion for an action; `null` when the person closes the dialog. */
  requestStepUp: (summary: string) => Promise<PasskeyAssertion | null>
  close: () => void
  enrol: (name: string) => Promise<void>
  verify: () => Promise<void>
}

/** Thrown inside a ceremony to end it with a message. */
class Refused extends Error {}

let authenticator: Authenticator | null | undefined
/** For tests: the authenticator the ceremonies use (default `navigator.credentials`). */
export function setAuthenticator(a: Authenticator | null | undefined): void {
  authenticator = a
}
function currentAuthenticator(): Authenticator | null {
  return authenticator === undefined ? browserAuthenticator() : authenticator
}

const waiting = new Map<RequestId, (event: Event) => void>()

onServerEvent(({ reply_to, event }) => {
  if (reply_to !== undefined) {
    const resolve = waiting.get(reply_to)
    if (resolve) {
      waiting.delete(reply_to)
      resolve(event)
    }
  } else if (event.type === 'PasskeyEnrolmentRequired') {
    usePasskeys.getState().openEnrol(true)
  }
})

/** Send `cmd` and wait for the event that answers it. */
function ask(cmd: Parameters<typeof sendCmd>[0]): Promise<Event> {
  const id = sendCmd(cmd)
  if (id === null) return Promise.reject(new Refused(OFFLINE))
  return new Promise((resolve) => waiting.set(id, resolve))
}

/** The message for an event that refused a passkey command (or one the command did not expect). */
export function refusalMessage(event: Event): string {
  if (event.type === 'PasskeyRefused') return REFUSED_MESSAGES[event.data.reason]
  if (event.type === 'PasskeyEnrolmentRequired') return ENROLMENT_REQUIRED
  if (event.type === 'OpError') return event.data.message
  if (event.type === 'Denied') return event.data.reason
  return `Unexpected answer from the server (${event.type}).`
}

function refusal(event: Event): Refused {
  return new Refused(refusalMessage(event))
}

/** Begin a ceremony and run the authenticator on its challenge. */
async function ceremony(
  purpose: PasskeyPurpose,
  onAuthenticator: () => void,
): Promise<{ ceremony: string; credential: PasskeyCredential }> {
  const auth = currentAuthenticator()
  if (!auth) throw new Refused(NO_AUTHENTICATOR)
  const challenge = await ask({ type: 'PasskeyBegin', data: { purpose } })
  if (challenge.type !== 'PasskeyChallenge') throw refusal(challenge)
  onAuthenticator()
  const { ceremony: id, options } = challenge.data
  let credential: PasskeyCredential
  try {
    if (purpose.kind === 'register') {
      const c = await auth.create(creationOptions(options))
      if (!c) throw new Refused(REFUSED_MESSAGES.cancelled)
      credential = attestationJson(c)
    } else {
      const c = await auth.get(requestOptions(options))
      if (!c) throw new Refused(REFUSED_MESSAGES.cancelled)
      credential = assertionJson(c)
    }
  } catch (e) {
    if (e instanceof Refused) throw e
    // NotAllowedError (cancelled or timed out) and the like: the prompt did not complete.
    throw new Refused(REFUSED_MESSAGES.cancelled)
  }
  return { ceremony: id, credential }
}

/** Resolves the pending `requestStepUp`, if any. */
let stepUpResolve: ((a: PasskeyAssertion | null) => void) | null = null
/** Bumped on every open and close, so an attempt from a closed dialog does nothing. */
let generation = 0

export const usePasskeys = create<PasskeyState>()((set, get) => {
  const open = (dialog: PasskeyDialog): void => {
    generation++
    set({ dialog, phase: 'idle', error: null })
  }

  /** Run `body` as the current attempt; failures become a message. */
  const attempt = async (body: (live: () => boolean) => Promise<void>): Promise<void> => {
    const gen = ++generation
    const live = () => gen === generation
    set({ phase: 'server', error: null })
    try {
      await body(live)
    } catch (e) {
      if (!live()) return
      set({ phase: 'refused', error: e instanceof Error ? e.message : String(e) })
    }
  }

  return {
    dialog: null,
    phase: 'idle',
    error: null,

    openEnrol: (required = false) => {
      const d = get().dialog
      // A required enrolment does not replace a ceremony in progress.
      if (d?.kind === 'enrol' && get().phase !== 'idle') return
      open({ kind: 'enrol', required })
    },
    openStepUp: () => {
      open({ kind: 'step_up', mode: 'verify', summary: null })
    },
    requestStepUp: (summary) => {
      stepUpResolve?.(null)
      open({ kind: 'step_up', mode: 'assert', summary })
      return new Promise((resolve) => {
        stepUpResolve = resolve
      })
    },
    close: () => {
      generation++
      stepUpResolve?.(null)
      stepUpResolve = null
      set({ dialog: null, phase: 'idle', error: null })
    },

    enrol: (name) =>
      attempt(async (live) => {
        const purpose: PasskeyPurpose = { kind: 'register', name: name.trim() }
        const { ceremony: id, credential } = await ceremony(purpose, () => {
          if (live()) set({ phase: 'authenticator' })
        })
        if (!live()) return
        set({ phase: 'server' })
        const done = await ask({ type: 'PasskeyFinish', data: { ceremony: id, credential } })
        if (!live()) return
        if (done.type !== 'PasskeyRegistered') throw refusal(done)
        set({ phase: 'done' })
      }),

    verify: () =>
      attempt(async (live) => {
        const d = get().dialog
        const assert = d?.kind === 'step_up' && d.mode === 'assert'
        const { ceremony: id, credential } = await ceremony({ kind: 'verify' }, () => {
          if (live()) set({ phase: 'authenticator' })
        })
        if (!live()) return
        if (assert) {
          if (!('authenticatorData' in credential.response)) throw new Refused(REFUSED_MESSAGES.invalid)
          const resolve = stepUpResolve
          stepUpResolve = null
          generation++
          set({ dialog: null, phase: 'idle', error: null })
          resolve?.({ ceremony: id, credential: credential as PasskeyAssertion['credential'] })
          return
        }
        set({ phase: 'server' })
        const done = await ask({ type: 'PasskeyFinish', data: { ceremony: id, credential } })
        if (!live()) return
        if (done.type !== 'Verification') throw refusal(done)
        set({ phase: 'done' })
      }),
  }
})

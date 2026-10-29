/**
 * Passkey wording shared by the dialogs and by every flow that steps up (spec 054). No side
 * effects, so the stacks, files, users and updates stores can import it freely.
 */
import type { PasskeyRefusedReason } from '@/protocol/generated'

export const REFUSED_MESSAGES: Record<PasskeyRefusedReason, string> = {
  replayed: 'That passkey answer was already used. Try again.',
  wrong_origin: 'The passkey answered for a different site.',
  unknown_credential: 'That passkey is not registered to you.',
  expired: 'The passkey check took too long and expired. Try again.',
  cancelled: 'The passkey prompt was cancelled.',
  recovery_invalid: 'That recovery code is not valid, or was already used.',
  invalid: 'The passkey answer was not valid. Try again.',
  last_passkey: 'This is your last passkey, so it cannot be removed. Add another first.',
}

export const NO_AUTHENTICATOR = 'This browser cannot use passkeys.'
export const OFFLINE = 'Depths is not connected. Wait for it to reconnect, then try again.'
export const NOT_CONFIRMED = 'Not confirmed, nothing changed.'
export const STEP_UP_AGAIN = 'This needs a fresh passkey check. Press Confirm to use your passkey.'
export const ENROL_FIRST = 'Register a passkey first. Nothing changed.'

/** A step-up that did not happen, as a flow's refusal carries it. */
export type StepUpRefusal =
  | { kind: 'not_confirmed' }
  | { kind: 'step_up' }
  | { kind: 'passkey'; reason: PasskeyRefusedReason | 'unsupported' }
  | { kind: 'enrol' }

export function stepUpText(r: StepUpRefusal): string {
  switch (r.kind) {
    case 'not_confirmed':
      return NOT_CONFIRMED
    case 'step_up':
      return STEP_UP_AGAIN
    case 'passkey':
      return r.reason === 'unsupported' ? `${NO_AUTHENTICATOR} Nothing changed.` : `${REFUSED_MESSAGES[r.reason]} Nothing changed.`
    case 'enrol':
      return ENROL_FIRST
  }
}

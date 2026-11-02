/**
 * Depths' failures as the kit's `Refusal` (spec 065 T021), so every list read fails the same way:
 * `LoadFailed` says "Failed to load {what}." with the reason, and Retry loads again.
 *
 * Depths' `Denied` names a reason, not an action, and the UI has always said it as a sentence
 * (`DENY_TEXT`). So a denial is an `error` refusal whose code is the reason and whose message is
 * that sentence: "You are not permitted to do this. (not_permitted)". `Invalid` and
 * `PasskeyRefused` follow the same shape. The server's strings are rendered as text, never markup.
 */
import type { Refusal } from '@companion/ui/kit'

import { OFFLINE as OFFLINE_TEXT, REFUSED_MESSAGES } from '@/features/passkeys/messages'
import { DENY_TEXT } from '@/features/stacks/argv'

import type { DenyReason, Event } from './generated'

export type { Refusal } from '@companion/ui/kit'

/** No socket, or the request never reached the server: Depths' side, said plainly. */
export const OFFLINE: Refusal = {
  kind: 'error',
  code: 'offline',
  message: OFFLINE_TEXT,
}

/** A `Denied{reason, detail?}` in Depths' words, with the server's detail after it when it sent one. */
export function deniedRefusal(reason: DenyReason, detail?: string): Refusal {
  const text = DENY_TEXT[reason]
  return { kind: 'error', code: reason, message: detail ? `${text} ${detail}` : text }
}

/** `Denied`, `Invalid` and `PasskeyRefused` as a `Refusal`; anything else is not one. */
export function refusalOf(event: Event): Refusal | null {
  switch (event.type) {
    case 'Denied':
      return deniedRefusal(event.data.reason, event.data.detail)
    case 'Invalid':
      return { kind: 'error', code: 'invalid', message: event.data.reason }
    case 'PasskeyRefused':
      return { kind: 'error', code: event.data.reason, message: REFUSED_MESSAGES[event.data.reason] }
    default:
      return null
  }
}

const HTTP_TEXT: Record<number, string> = {
  401: 'Your session ended. Sign in again.',
  403: 'You are not permitted to do this.',
  404: 'The server does not know this page.',
}

/**
 * A failed HTTP read (`ApiResult`'s `{status, error}`) as a `Refusal`. Status 0 is a request that
 * never got an answer, which is `OFFLINE`. A `403` naming a deny reason reads as that denial.
 */
export function httpRefusal(status: number, error: string): Refusal {
  if (status === 0) return OFFLINE
  if (status === 403 && error in DENY_TEXT) return deniedRefusal(error as DenyReason)
  return {
    kind: 'error',
    code: error === 'error' ? `http_${String(status)}` : error,
    message: HTTP_TEXT[status] ?? `The server answered ${String(status)}.`,
  }
}

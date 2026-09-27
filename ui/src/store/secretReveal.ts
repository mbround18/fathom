/**
 * #40: on-demand secret value reveal. SECURITY: values never enter the zustand store (so they
 * never reach devtools, persistence, or a re-render of unrelated state). The reducer hands a
 * `SecretValueRevealed` straight to this module, keyed by the request that asked for it; the
 * caller awaiting that request takes it (and it is forgotten) as soon as the command finishes.
 * Unclaimed answers are dropped after a few seconds.
 */
import type { Correlation, SecretRevealOutcome, SecretValueReveal } from '@/protocol/types'

import { waitForCommandResult } from './actions'
import { useStore } from './index'

/** How long an answer nobody has claimed yet is kept. */
const UNCLAIMED_MS = 10_000

const answers = new Map<number, SecretRevealOutcome>()

/** Called by the reducer for every `SecretValueRevealed` (never stored anywhere else). */
export function secretRevealed(correlation: Correlation, reveal: SecretValueReveal): void {
  if (correlation.type !== 'Request') return
  const id = correlation.data
  answers.set(id, reveal.outcome)
  setTimeout(() => {
    answers.delete(id)
  }, UNCLAIMED_MS)
}

/** What a reveal came to: the backend's typed outcome, or why there is none. */
export type RevealResult = SecretRevealOutcome | { type: 'NoAnswer'; data: { message: string } }

/**
 * Ask the backend for one key of one Secret (`RevealSecretValue`) and wait for the answer.
 * The value is returned to the caller only; nothing is cached.
 */
export async function revealSecretValue(
  namespace: string,
  name: string,
  key: string,
): Promise<RevealResult> {
  let requestId: number
  try {
    requestId = await useStore.getState().request({
      type: 'RevealSecretValue',
      data: { namespace, name, key },
    })
  } catch (err) {
    return noAnswer(err instanceof Error ? err.message : String(err))
  }
  const result = await waitForCommandResult(requestId)
  const outcome = answers.get(requestId)
  answers.delete(requestId)
  if (outcome) return outcome
  if (result === 'timeout') return noAnswer('The backend did not answer in time.')
  if (!result.ok) return { type: 'Failed', data: { message: result.error ?? 'Reveal failed.' } }
  return noAnswer('The backend sent no value (the context may have changed).')
}

function noAnswer(message: string): RevealResult {
  return { type: 'NoAnswer', data: { message } }
}

/** Tests: forget every unclaimed answer. */
export function resetSecretReveals(): void {
  answers.clear()
}

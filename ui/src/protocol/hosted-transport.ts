/**
 * Hosted-server transport (spec 031, T029). Placeholder so the `hosted` transport kind
 * type-checks; lane C replaces this with the same-origin cookie-authenticated WebSocket
 * transport.
 */
import type { Transport } from './transport'

export function createHostedTransport(): Promise<Transport> {
  return Promise.reject(new Error('hosted transport not implemented yet (T029)'))
}

/**
 * Spec 052 in the UI: what the restricted view allows, and the words for why the rest is refused.
 * The server decides (it gates every command and route after the role check); these only keep the
 * controls honest about it.
 */
import { useSession } from '@/app/session-store'
import type { NetworkAnswer, NetworkSource, OperationKind, TrustDecision, TrustReason } from '@/protocol/generated'

/** Operations an untrusted network may still run (FR-009): start, stop, restart. */
export const RESTRICTED_OPS: ReadonlySet<OperationKind> = new Set<OperationKind>([
  'up',
  'stop',
  'restart',
  'service_up',
  'service_stop',
  'service_restart',
])

/** `YYYY-MM-DD` of an RFC 3339 time, so the text is the same in every locale and test. */
export function day(rfc3339: string): string {
  return rfc3339.slice(0, 10)
}

/** When, if ever, this network would become trusted. */
export function untrustedText(d: Pick<TrustDecision, 'reason' | 'matures_at'>): string {
  switch (d.reason) {
    case 'maturing':
      return d.matures_at
        ? `This network is trusted from ${day(d.matures_at)}, once you have used it on two separate days.`
        : 'This network becomes trusted once you have used it on two separate days.'
    case 'public':
      return 'You said this network is public, so it stays restricted. An administrator can whitelist it.'
    default:
      return 'This network is new. Say whether it is home or public; it is restricted either way for now.'
  }
}

export const REASON_LABEL: Record<TrustReason, string> = {
  always: 'Trusted (always-trusted list)',
  first: 'Trusted (first network)',
  whitelist: 'Trusted (whitelisted)',
  matured: 'Trusted (home, matured)',
  gate_off: 'Trusted (gate off)',
  unknown: 'Restricted (not answered)',
  public: 'Restricted (public)',
  maturing: 'Restricted (home, maturing)',
}

export const ANSWER_LABEL: Record<NetworkAnswer, string> = { home: 'Home', public: 'Public', none: 'Not answered' }

export const SOURCE_LABEL: Record<NetworkSource, string> = {
  first: 'First network',
  answer: 'Answered',
  whitelist: 'Whitelisted',
  seen: 'Seen',
}

/** The title on a refused control: `null` when this network is trusted. */
export function blockedText(d: TrustDecision | undefined): string | null {
  if (!d || d.trusted) return null
  return `Not from this network. ${untrustedText(d)}`
}

/** [`blockedText`] for the session's current network. */
export function useUntrusted(): string | null {
  return useSession((s) => blockedText(s.me?.network))
}

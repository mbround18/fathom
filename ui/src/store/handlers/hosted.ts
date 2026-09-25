/** 031-hosted-server-mode: shell mode, identity, limits and the terminal hosted states. */
import type {
  LimitKind,
  PublicIdentity,
  PublicLimits,
  SessionEndReason,
  ShellMode,
  SubscriptionInfo,
} from '@/protocol/types'

export interface HostedState {
  mode: ShellMode
  identity?: PublicIdentity
  limits?: PublicLimits
  /** Set by `SystemEvent::SessionEnded`; the app shows the sign-in-again screen. */
  sessionEnded?: SessionEndReason
  /** Latest `SystemEvent::LimitReached`; dismissible. */
  limitReached?: { kind: LimitKind; max: number }
}

/** Fold a (possibly repeated) `SubscriptionInfo` into the hosted slice. */
export function applySubscriptionInfo(prev: HostedState, info: SubscriptionInfo): HostedState {
  const next: HostedState = { ...prev, mode: info.mode ?? 'desktop' }
  if (info.identity) next.identity = info.identity
  if (info.limits) next.limits = info.limits
  return next
}

export const isHosted = (h: HostedState): boolean => h.mode === 'hosted'

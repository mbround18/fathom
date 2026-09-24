/** 034-simultaneous-federation: `FederationStateUpdated` / `FederationRollupUpdated` handling. */
import type { FederationCombinedRollup, FederationContextRollup } from '@/protocol/types'

import type { Draft } from '../draft'

export interface FederationSlice {
  /** Contexts in the federated set (sorted). */
  federated: string[]
  softCap: number
  overCap: boolean
  /** Latest rollup per federated context. */
  rollups: Record<string, FederationContextRollup>
  combined: FederationCombinedRollup | null
}

export const initialFederation = (): FederationSlice => ({
  federated: [],
  softCap: 8,
  overCap: false,
  rollups: {},
  combined: null,
})

/** The set changed: adopt it and drop rollups of contexts that left. */
export function applyFederationState(
  d: Draft,
  data: { federated: string[]; soft_cap: number; over_cap: boolean },
): void {
  const session = d.edit('session')
  const keep = new Set(data.federated)
  const rollups: Record<string, FederationContextRollup> = {}
  for (const [ctx, r] of Object.entries(session.federation.rollups)) {
    if (keep.has(ctx)) rollups[ctx] = r
  }
  session.federation = {
    ...session.federation,
    federated: data.federated,
    softCap: data.soft_cap,
    overCap: data.over_cap,
    rollups,
    combined: data.federated.length === 0 ? null : session.federation.combined,
  }
}

/** Events older than the newest generation seen for that context are dropped. */
export function applyFederationRollup(
  d: Draft,
  data: { context: FederationContextRollup; combined: FederationCombinedRollup },
): void {
  const session = d.edit('session')
  const prev = session.federation.rollups[data.context.context]
  if (prev && prev.generation > data.context.generation) return
  session.federation = {
    ...session.federation,
    rollups: { ...session.federation.rollups, [data.context.context]: data.context },
    combined: data.combined,
  }
}

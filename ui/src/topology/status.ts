/**
 * Node health/status color (research.md R6): reuses the same `statusTone` classifier every list
 * view already uses (pod phase, workload ready-vs-desired), so graph colors and list-row colors
 * never drift apart into a second, independently-maintained notion of "healthy".
 */
// `@/store` must be imported before any descriptor module that (transitively, via a workload
// descriptor's `@/store` import for pod drill-through) imports `@/store` itself — otherwise the
// reentrant cycle leaves that descriptor's module mid-evaluation (see tests/unit/actions.test.ts).
import '@/store'

import { workloadStatus } from '@/resources/workloads'
import { statusTone, type Tone } from '@/resources/search'
import type { NodeSummary, PodSummary, WorkloadSummary } from '@/protocol/types'

export function podTone(p: Pick<PodSummary, 'phase'>): Tone {
  return statusTone(p.phase).tone
}

export function workloadTone(w: WorkloadSummary): Tone {
  return statusTone(workloadStatus(w)).tone
}

export function nodeSummaryTone(n: NodeSummary): Tone {
  return statusTone(n.status).tone
}

/** Namespace, Application, HelmRelease nodes have no single "phase" of their own in v1 — neutral. */
export const NEUTRAL_TONE: Tone = 'muted'

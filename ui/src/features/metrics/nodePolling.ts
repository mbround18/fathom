/**
 * Shared, reference-counted node-metrics polling. The backend runs one node poll at a time, so
 * every UI surface that shows node usage (Nodes list cells, the Node inspector) acquires it
 * here: the first acquire sends `StartNodeMetricsPolling`, and the poll stops shortly after the
 * last release, i.e. only while something showing node metrics is on screen.
 */
import { useEffect } from 'react'

import type { NodeMetricsSummary } from '@/protocol/types'
import { useStore } from '@/store'
import { detailKey } from '@/store/keys'

export const NODE_POLL_SECS = 5
/** Grace period so virtualised rows scrolling in and out do not restart the poll. */
const STOP_DELAY_MS = 1500

let holders = 0
let stopTimer: ReturnType<typeof setTimeout> | undefined

export function acquireNodeMetricsPolling(): () => void {
  const { send } = useStore.getState()
  if (stopTimer !== undefined) {
    clearTimeout(stopTimer)
    stopTimer = undefined
  }
  if (holders === 0) {
    send({ type: 'StartNodeMetricsPolling', data: { interval_secs: NODE_POLL_SECS } })
  }
  holders += 1
  let released = false
  return () => {
    if (released) return
    released = true
    holders -= 1
    if (holders === 0) {
      stopTimer = setTimeout(() => {
        stopTimer = undefined
        if (holders === 0) useStore.getState().send({ type: 'StopNodeMetricsPolling' })
      }, STOP_DELAY_MS)
    }
  }
}

/** Poll node metrics for as long as the calling component is mounted. */
export function useNodeMetricsPolling(): void {
  useEffect(() => acquireNodeMetricsPolling(), [])
}

interface NodeMetricsSource {
  details: Record<string, { eventType: string; data: unknown }>
  session: { activeContext?: string | undefined }
}

/** Latest node metrics for `name` in the active context, from the detail cache. */
export function nodeMetricsOf(s: NodeMetricsSource, name: string): NodeMetricsSummary | undefined {
  const d = s.details[detailKey(s.session.activeContext, 'NodeMetrics', null, name)]
  return d?.eventType === 'NodeMetricsUpdated' ? (d.data as NodeMetricsSummary) : undefined
}

export function useNodeMetrics(name: string): NodeMetricsSummary | undefined {
  return useStore((s) => nodeMetricsOf(s, name))
}

/** Non-reactive lookup for column `text`/`sort`, which run outside React. */
export function nodeMetricsNow(name: string): NodeMetricsSummary | undefined {
  return nodeMetricsOf(useStore.getState(), name)
}

export type NodeUsageMetric = 'cpu' | 'memory'

export function nodeUsagePercent(
  m: NodeMetricsSummary | undefined,
  metric: NodeUsageMetric,
): number | null {
  const p = metric === 'cpu' ? m?.cpu.usage_percent : m?.memory.usage_percent
  return p ?? null
}

export function nodeUsageText(m: NodeMetricsSummary | undefined, metric: NodeUsageMetric): string {
  const p = nodeUsagePercent(m, metric)
  return p === null ? '—' : `${p.toFixed(1)}%`
}

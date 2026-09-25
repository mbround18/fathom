import { useEffect, useRef, useState } from 'react'

import type { PrometheusMetrics, WorkloadMetricsAggregate } from '@/protocol/types'
import { useStore } from '@/store'

const WORKLOAD_EVENTS = new Set([
  'DeploymentMetricsUpdated',
  'DaemonSetMetricsUpdated',
  'StatefulSetMetricsUpdated',
  'ApplicationMetricsUpdated',
])

/** Latest Prometheus pod metrics events, newest first. */
export function usePodMetrics(): PrometheusMetrics[] {
  const details = useStore((s) => s.details)
  return Object.values(details)
    .filter((d) => d.eventType === 'PrometheusMetricsUpdated')
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .map((d) => d.data as PrometheusMetrics)
}

export function useWorkloadMetrics(): WorkloadMetricsAggregate[] {
  const details = useStore((s) => s.details)
  return Object.values(details)
    .filter((d) => WORKLOAD_EVENTS.has(d.eventType))
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .map((d) => d.data as WorkloadMetricsAggregate)
}

/** Rolling history (max `cap`) of a value; one point per distinct `stamp`. Drives sparklines. */
export function useHistory(value: number | null | undefined, stamp: number, cap = 40): number[] {
  const [hist, setHist] = useState<number[]>([])
  const last = useRef<number>(-1)
  useEffect(() => {
    if (value == null || stamp === last.current) return
    last.current = stamp
    setHist((h) => [...h.slice(-(cap - 1)), value])
  }, [value, stamp, cap])
  return hist
}

export function fmtBytes(n: number | null | undefined): string {
  if (n == null) return 'n/a'
  const u = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let v = n
  let i = 0
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${u[i] ?? 'B'}`
}

export function fmtPct(n: number | null | undefined): string {
  return n == null ? 'n/a' : `${n.toFixed(1)}%`
}

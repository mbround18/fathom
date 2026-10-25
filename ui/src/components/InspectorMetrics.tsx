/**
 * Metrics section of the inspector Details tab (T105) for nodes, pods and workloads, fed by the
 * same store events the Metrics tool uses. Nodes and workloads are polled by the backend only
 * while this section is mounted (i.e. while the inspector is open on them); when the metrics
 * API is absent no events arrive and the metrics-unavailable state stays.
 */
import { useEffect } from 'react'

import { MetricsChart, type ChartPoint } from '@/components/MetricsChart'
import { MetricsUnavailable } from '@companion/terminal'
import { useNodeMetrics, useNodeMetricsPolling } from '@/features/metrics/nodePolling'
import {
  fmtBytes,
  fmtPct,
  useHistory,
  usePodMetrics,
  useWorkloadMetrics,
} from '@/features/metrics/selectors'
import { useStore } from '@/store'
import { detailKey } from '@/store/keys'

export const METRICS_KINDS: ReadonlySet<string> = new Set([
  'Node',
  'Pod',
  'Deployment',
  'StatefulSet',
  'DaemonSet',
])

const WORKLOAD_POLL_SECS = 5

interface Target {
  kind: string
  namespace?: string | null | undefined
  name: string
}

function toPoints(cpu: number[], mem: number[]): ChartPoint[] {
  const n = Math.max(cpu.length, mem.length)
  return Array.from({ length: n }, (_, i) => ({
    t: i,
    cpu: cpu[i] ?? null,
    memoryMiB: mem[i] === undefined ? null : mem[i] / 1048576,
  }))
}

function Charts({
  cpuNow,
  memNow,
  stamp,
  label,
}: {
  cpuNow: number | null | undefined
  memNow: number | null | undefined
  stamp: number
  label: string
}) {
  const cpu = useHistory(cpuNow, stamp)
  const mem = useHistory(memNow, stamp)
  return (
    <div className="flex flex-col gap-2" data-testid="inspector-metrics">
      <p className="font-mono text-xs">
        CPU {fmtPct(cpuNow)} · Memory {fmtBytes(memNow)}
      </p>
      <MetricsChart
        label={label}
        series={[
          { key: 'cpu', label: 'CPU %' },
          { key: 'memoryMiB', label: 'Memory MiB' },
        ]}
        points={toPoints(cpu, mem)}
      />
    </div>
  )
}

/** Poll a workload's aggregate metrics while mounted, once its details supply the pod selector. */
function useWorkloadPolling(target: Target) {
  const send = useStore((s) => s.send)
  const { kind, namespace, name } = target
  const selector = useStore((s) => {
    const d = s.details[detailKey(s.session.activeContext, kind, namespace, name)]
    const sel = (d?.data as { pod_selector?: unknown } | undefined)?.pod_selector
    return typeof sel === 'string' ? sel : null
  })
  useEffect(() => {
    if (selector === null || !namespace) return
    send({
      type: 'StartWorkloadMetricsPolling',
      data: {
        namespace,
        workload_name: name,
        workload_kind: kind,
        pod_selector: selector,
        interval_secs: WORKLOAD_POLL_SECS,
      },
    })
    return () => {
      send({ type: 'StopWorkloadMetricsPolling' })
    }
  }, [send, selector, kind, namespace, name])
}

function NodeMetricsView({ name }: { name: string }) {
  useNodeMetricsPolling()
  const m = useNodeMetrics(name)
  if (!m) {
    return (
      <MetricsUnavailable detail="No node metrics yet. They appear once metrics-server reports this node; if it is not installed or not permitted they stay unavailable." />
    )
  }
  return (
    <Charts
      cpuNow={m.cpu.usage_percent}
      memNow={m.memory.usage_bytes}
      stamp={m.timestamp}
      label={`${name} CPU and memory history`}
    />
  )
}

function WorkloadMetricsView({ target }: { target: Target }) {
  useWorkloadPolling(target)
  const workloads = useWorkloadMetrics()
  const w = workloads.find(
    (x) =>
      x.workload_kind === target.kind && x.namespace === target.namespace && x.name === target.name,
  )
  if (!w) return <MetricsUnavailable detail="No workload metrics for this resource yet." />
  if (w.total_cpu_usage_percent == null && w.total_memory_usage_bytes == null) {
    return (
      <MetricsUnavailable detail="No usage data for this workload: metrics-server is not installed or not reachable, and no Prometheus integration is configured." />
    )
  }
  return (
    <Charts
      cpuNow={w.total_cpu_usage_percent}
      memNow={w.total_memory_usage_bytes}
      stamp={w.timestamp}
      label={`${target.name} CPU and memory history`}
    />
  )
}

export function InspectorMetrics({ target }: { target: Target }) {
  const pods = usePodMetrics()
  if (target.kind === 'Node') return <NodeMetricsView name={target.name} />
  if (target.kind !== 'Pod') return <WorkloadMetricsView target={target} />
  const m = pods.find((p) => p.namespace === target.namespace && p.name === target.name)
  if (!m) {
    return (
      <MetricsUnavailable detail="No metrics for this pod yet. Start polling it in the Metrics tool." />
    )
  }
  return (
    <Charts
      cpuNow={m.cpu.usage_percent}
      memNow={m.memory.usage_bytes}
      stamp={m.timestamp}
      label={`${target.name} CPU and memory history`}
    />
  )
}

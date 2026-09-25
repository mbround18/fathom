/**
 * Metrics section of the inspector Details tab (T105) for pods and workloads, fed by the same
 * store events the Metrics tool uses. Nodes are reported honestly as unavailable: the backend
 * defines `NodeMetricsSummary` but does not emit node metrics events yet.
 */
import { MetricsChart, type ChartPoint } from '@/components/MetricsChart'
import { MetricsUnavailable } from '@/components/shell/PaneStates'
import {
  fmtBytes,
  fmtPct,
  useHistory,
  usePodMetrics,
  useWorkloadMetrics,
} from '@/features/metrics/selectors'

export const METRICS_KINDS: ReadonlySet<string> = new Set([
  'Node',
  'Pod',
  'Deployment',
  'StatefulSet',
  'DaemonSet',
])

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

export function InspectorMetrics({ target }: { target: Target }) {
  const pods = usePodMetrics()
  const workloads = useWorkloadMetrics()
  if (target.kind === 'Node') {
    return (
      <MetricsUnavailable detail="Node metrics are not provided by the backend yet; see the Metrics tool for pod and workload metrics." />
    )
  }
  if (target.kind === 'Pod') {
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
  const w = workloads.find(
    (x) =>
      x.workload_kind === target.kind && x.namespace === target.namespace && x.name === target.name,
  )
  if (!w) return <MetricsUnavailable detail="No workload metrics for this resource yet." />
  return (
    <Charts
      cpuNow={w.total_cpu_usage_percent}
      memNow={w.total_memory_usage_bytes}
      stamp={w.timestamp}
      label={`${target.name} CPU and memory history`}
    />
  )
}

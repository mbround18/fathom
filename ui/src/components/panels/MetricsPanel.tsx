import { useEffect, useState } from 'react'

import { MetricsUnavailable } from '@/components/shell/PaneStates'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { PrometheusForm } from '@/features/metrics/PrometheusForm'
import { prometheusStatusWord } from '@/features/metrics/prometheusStatusText'
import {
  fmtBytes,
  fmtPct,
  useHistory,
  usePodMetrics,
  useWorkloadMetrics,
} from '@/features/metrics/selectors'
import { Bar, Sparkline } from '@/features/metrics/Sparkline'
import type { PrometheusMetrics } from '@/protocol/types'
import { useStore } from '@/store'

function PodRow({ m }: { m: PrometheusMetrics }) {
  const cpu = useHistory(m.cpu.usage_percent, m.timestamp)
  const mem = useHistory(m.memory.usage_bytes, m.timestamp)
  const id = `${m.namespace}/${m.name}`
  return (
    <tr data-testid="pod-metrics-row">
      <td className="p-1">{id}</td>
      <td className="p-1">
        <Bar percent={m.cpu.usage_percent} label={`${id} CPU`} />
        {fmtPct(m.cpu.usage_percent)} <Sparkline values={cpu} label={`${id} CPU history`} />
      </td>
      <td className="p-1">
        <Bar percent={m.memory.usage_percent} label={`${id} memory`} />
        {fmtBytes(m.memory.usage_bytes)} <Sparkline values={mem} label={`${id} memory history`} />
      </td>
    </tr>
  )
}

export default function MetricsPanel() {
  const send = useStore((s) => s.send)
  const test = useStore((s) => s.integrations.prometheusTest)
  const noContext = useStore((s) => s.session.activeContext === undefined)
  const pods = usePodMetrics()
  const workloads = useWorkloadMetrics()
  const [ns, setNs] = useState('')
  const [name, setName] = useState('')
  const [polling, setPolling] = useState(false)

  // Polling is tied to panel lifetime: unmount (or Stop) stops it.
  useEffect(() => {
    if (!polling) return
    send({
      type: 'StartMetricsPolling',
      data: { namespace: ns.trim(), name: name.trim(), interval_secs: 5 },
    })
    return () => {
      send({ type: 'StopMetricsPolling' })
    }
    // Target edits apply on the next Start.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [polling, send])

  if (noContext) return <MetricsUnavailable detail="Select a cluster to view metrics." />

  const status = prometheusStatusWord(test)
  const targetOk = ns.trim() !== '' && name.trim() !== ''
  return (
    <div className="flex h-full flex-col overflow-auto">
      <p className="p-3 text-xs" data-testid="metrics-status">
        Prometheus: {status}
      </p>
      <div className="flex flex-wrap items-end gap-2 px-3">
        <div className="flex flex-col gap-1 text-xs">
          <span>Pod namespace</span>
          <Input aria-label="Pod namespace" value={ns} onChange={(e) => setNs(e.target.value)} />
        </div>
        <div className="flex flex-col gap-1 text-xs">
          <span>Pod name</span>
          <Input aria-label="Pod name" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <Button disabled={!targetOk && !polling} onClick={() => setPolling((p) => !p)}>
          {polling ? 'Stop polling' : 'Start polling'}
        </Button>
        <Button
          variant="outline"
          disabled={!targetOk}
          onClick={() =>
            send({ type: 'FetchPodMetrics', data: { namespace: ns.trim(), name: name.trim() } })
          }
        >
          Fetch once
        </Button>
      </div>
      {pods.length === 0 && workloads.length === 0 ? (
        <MetricsUnavailable
          detail={
            test?.success
              ? 'No metrics received yet. Enter a pod, then Fetch once or Start polling.'
              : 'No metrics received yet. Configure Prometheus below, then start polling a pod.'
          }
        />
      ) : (
        <table className="m-3 text-xs">
          <thead>
            <tr>
              <th className="p-1 text-left">Target</th>
              <th className="p-1 text-left">CPU</th>
              <th className="p-1 text-left">Memory</th>
            </tr>
          </thead>
          <tbody>
            {pods.map((m) => (
              <PodRow key={`${m.namespace}/${m.name}`} m={m} />
            ))}
            {workloads.map((w) => (
              <tr
                key={`${w.workload_kind}/${w.namespace}/${w.name}`}
                data-testid="workload-metrics-row"
              >
                <td className="p-1">
                  {w.workload_kind} {w.namespace}/{w.name} ({w.replica_count})
                </td>
                <td className="p-1">
                  <Bar percent={w.total_cpu_usage_percent} label={`${w.name} CPU`} />
                  {fmtPct(w.total_cpu_usage_percent)}
                </td>
                <td className="p-1">{fmtBytes(w.total_memory_usage_bytes)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <PrometheusForm />
    </div>
  )
}

/**
 * Host metrics for admins (`WatchHost`): a panel of meters with sparklines on the overview, and a
 * one-line strip above the stack list on narrow screens. Neither renders for anyone else, nor
 * when the server has host metrics off (`me.features.host_metrics`).
 */
import { useEffect } from 'react'

import { useHost, watchHost } from './host-store'
import { formatBytes, formatPercent, formatUptime, hostPoint, sparkPath, type HostPoint } from './metrics'

/** Watches the host while mounted. */
function useHostWatch(): void {
  useEffect(() => watchHost(), [])
}

const SPARK_W = 120
const SPARK_H = 24

function Sparkline({ values, label }: { values: (number | null)[]; label: string }) {
  return (
    <svg
      role="img"
      aria-label={`${label}, last ${String(values.length)} samples`}
      viewBox={`0 0 ${String(SPARK_W)} ${String(SPARK_H)}`}
      preserveAspectRatio="none"
      className="h-6 w-full text-primary"
    >
      <path d={sparkPath(values, SPARK_W, SPARK_H)} fill="none" stroke="currentColor" strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

function Meter({ label, value }: { label: string; value: number | null }) {
  const pct = value === null ? 0 : Math.max(0, Math.min(100, value))
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(pct)}
      className="h-1 w-full bg-muted"
    >
      <div className={`h-full ${pct >= 90 ? 'bg-status-err' : 'bg-primary'}`} style={{ width: `${String(pct)}%` }} />
    </div>
  )
}

function Figure({
  name,
  label,
  value,
  detail,
  meter,
  history,
}: {
  name: keyof HostPoint
  label: string
  value: string
  detail: string
  meter: number | null
  history: (number | null)[]
}) {
  return (
    <div data-host={name} className="flex min-w-0 flex-col gap-1 border border-border bg-card p-3">
      <div className="flex min-w-0 items-baseline justify-between gap-2">
        <span className="text-xs text-muted-foreground">{label}</span>
        <span data-field="value" className="font-display text-lg font-semibold tabular-nums">
          {value}
        </span>
      </div>
      <Meter label={label} value={meter} />
      <Sparkline values={history} label={`${label} history`} />
      <span data-field="detail" className="truncate text-xs text-muted-foreground tabular-nums">
        {detail}
      </span>
    </div>
  )
}

/** The overview's host panel. */
export function HostPanel() {
  useHostWatch()
  const latest = useHost((s) => s.latest)
  const history = useHost((s) => s.history)
  const denied = useHost((s) => s.denied)

  if (denied) {
    return (
      <p role="status" data-host-state="denied" className="text-xs text-muted-foreground">
        Host metrics are unavailable ({denied.replace('_', ' ')}).
      </p>
    )
  }
  const point = latest ? hostPoint(latest) : null
  const series = (k: keyof HostPoint) => history.map((p) => p[k])
  return (
    <section data-section="host" aria-labelledby="host-title" className="flex min-w-0 flex-col gap-2">
      <div className="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3">
        <h2 id="host-title" className="font-display text-xs font-semibold tracking-wider uppercase">
          Host
        </h2>
        {latest && (
          <span data-field="host-summary" className="text-xs text-muted-foreground tabular-nums">
            up {formatUptime(latest.uptime_secs)} · {latest.containers_running} running · {latest.containers_stopped} stopped
          </span>
        )}
      </div>
      {!latest ? (
        <p role="status" data-host-state="loading" className="text-xs text-muted-foreground">
          Reading host metrics…
        </p>
      ) : (
        <div className="grid min-w-0 grid-cols-1 gap-2 sm:grid-cols-2 2xl:grid-cols-4">
          <Figure
            name="cpu"
            label="CPU"
            value={formatPercent(point?.cpu)}
            detail={latest.cpu_cores ? `${String(latest.cpu_cores)} cores` : '—'}
            meter={point?.cpu ?? null}
            history={series('cpu')}
          />
          <Figure
            name="memory"
            label="Memory"
            value={formatPercent(point?.memory)}
            detail={
              latest.memory
                ? `${formatBytes(latest.memory.used_bytes)} of ${formatBytes(latest.memory.total_bytes)}` +
                  (latest.swap && latest.swap.total_bytes > 0 ? ` · swap ${formatBytes(latest.swap.used_bytes)}` : '')
                : '—'
            }
            meter={point?.memory ?? null}
            history={series('memory')}
          />
          <Figure
            name="disk"
            label="Disk"
            value={formatPercent(point?.disk)}
            detail={latest.disk ? `${formatBytes(latest.disk.free_bytes)} free of ${formatBytes(latest.disk.total_bytes)}` : '—'}
            meter={point?.disk ?? null}
            history={series('disk')}
          />
          <Figure
            name="load"
            label="Load"
            value={latest.load ? latest.load.one.toFixed(2) : '—'}
            detail={latest.load ? `5m ${latest.load.five.toFixed(2)} · 15m ${latest.load.fifteen.toFixed(2)}` : '—'}
            meter={point?.load ?? null}
            history={series('load')}
          />
        </div>
      )}
    </section>
  )
}

/** One line of host figures, above the stack list on narrow screens. */
export function HostStrip() {
  useHostWatch()
  const latest = useHost((s) => s.latest)
  const denied = useHost((s) => s.denied)
  if (denied || !latest) return null
  const p = hostPoint(latest)
  const item = (name: string, label: string, value: string) => (
    <span data-host={name} className="whitespace-nowrap">
      <span className="text-muted-foreground">{label}</span> <span className="tabular-nums">{value}</span>
    </span>
  )
  return (
    <div
      data-section="host-strip"
      aria-label="Host"
      role="group"
      className="flex min-w-0 shrink-0 flex-wrap gap-x-3 gap-y-0.5 border-b border-border px-2 py-1 text-xs"
    >
      {item('cpu', 'CPU', formatPercent(p.cpu))}
      {item('memory', 'Mem', formatPercent(p.memory))}
      {item('disk', 'Disk', formatPercent(p.disk))}
      {item('load', 'Load', latest.load ? latest.load.one.toFixed(2) : '—')}
    </div>
  )
}

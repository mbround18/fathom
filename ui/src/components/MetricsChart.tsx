/**
 * Recharts line chart themed with `--chart-1..5`, a monospace legend, and no animation under
 * `prefers-reduced-motion` (T103). Data is `points[i][seriesKey]`; a missing value leaves a gap.
 */
import { useSyncExternalStore } from 'react'
import { Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'

export interface ChartSeries {
  key: string
  label: string
}

export type ChartPoint = Record<string, number | null> & { t: number }

const QUERY = '(prefers-reduced-motion: reduce)'

function subscribe(cb: () => void): () => void {
  if (typeof matchMedia !== 'function') return () => undefined
  const m = matchMedia(QUERY)
  m.addEventListener('change', cb)
  return () => m.removeEventListener('change', cb)
}

export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => typeof matchMedia === 'function' && matchMedia(QUERY).matches,
    () => false,
  )
}

export function MetricsChart({
  series,
  points,
  label,
  height = 140,
  format,
}: {
  series: readonly ChartSeries[]
  points: readonly ChartPoint[]
  /** Text alternative for the chart. */
  label: string
  height?: number
  format?: (v: number) => string
}) {
  const reduced = usePrefersReducedMotion()
  if (points.length < 2) {
    return <p className="text-xs text-muted-foreground">Collecting samples…</p>
  }
  return (
    <div
      role="img"
      aria-label={label}
      data-testid="metrics-chart"
      data-animated={reduced ? 'false' : 'true'}
      style={{ height }}
      className="w-full min-w-0 font-mono text-[10px]"
    >
      <ResponsiveContainer width="100%" height="100%" initialDimension={{ width: 300, height }}>
        <LineChart data={points}>
          <XAxis dataKey="t" hide />
          <YAxis width={44} tick={{ fontSize: 10 }} tickFormatter={format} />
          <Tooltip formatter={(v) => (typeof v === 'number' && format ? format(v) : String(v))} />
          <Legend wrapperStyle={{ fontFamily: 'var(--font-mono, monospace)', fontSize: 10 }} />
          {series.map((s, i) => (
            <Line
              key={s.key}
              type="monotone"
              dataKey={s.key}
              name={s.label}
              stroke={`var(--chart-${String((i % 5) + 1)})`}
              dot={false}
              strokeWidth={1.5}
              isAnimationActive={!reduced}
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}

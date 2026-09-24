/** 036 US5: dependency-free trend line with an explicit marker when older samples were dropped. */
import type { DaemonMetricSample } from '@/protocol/types'

import { formatTime } from './state'

export interface SparklineProps {
  samples: readonly DaemonMetricSample[]
  /** ISO time the retained history starts at, set when the daemon's ring dropped older data. */
  truncatedOldestTime: string | null
  label: string
  unit?: string
}

const W = 100
const H = 24

export function sparklinePoints(samples: readonly DaemonMetricSample[]): string {
  if (samples.length === 0) return ''
  const values = samples.map((s) => s.value)
  const min = Math.min(...values)
  const max = Math.max(...values)
  const span = max - min || 1
  return samples
    .map((s, i) => {
      const x = samples.length === 1 ? W / 2 : (i / (samples.length - 1)) * W
      const y = H - 2 - ((s.value - min) / span) * (H - 4)
      return `${x.toFixed(2)},${y.toFixed(2)}`
    })
    .join(' ')
}

export function Sparkline({ samples, truncatedOldestTime, label, unit }: SparklineProps) {
  const last = samples.at(-1)
  const summary =
    last === undefined
      ? `${label}: no samples`
      : `${label}: latest ${String(Math.round(last.value * 100) / 100)}${unit ? ` ${unit}` : ''}, ${String(samples.length)} samples`
  return (
    <figure className="m-0 flex min-w-0 flex-col gap-0.5" data-testid="daemon-sparkline">
      <figcaption className="text-xs text-muted-foreground">{summary}</figcaption>
      <svg
        role="img"
        aria-label={summary}
        viewBox={`0 0 ${String(W)} ${String(H)}`}
        preserveAspectRatio="none"
        className="h-8 w-full text-accent-neon"
      >
        {truncatedOldestTime !== null && (
          <line
            data-testid="daemon-sparkline-truncation"
            x1="0.5"
            x2="0.5"
            y1="0"
            y2={H}
            stroke="currentColor"
            strokeDasharray="2 2"
            className="text-status-warn"
            vectorEffect="non-scaling-stroke"
          />
        )}
        <polyline
          points={sparklinePoints(samples)}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      {truncatedOldestTime !== null && (
        <span className="text-xs text-status-warn" data-testid="daemon-sparkline-note">
          History starts {formatTime(truncatedOldestTime)}; older samples were dropped.
        </span>
      )}
    </figure>
  )
}

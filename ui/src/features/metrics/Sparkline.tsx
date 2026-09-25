/** Tiny accessible sparkline; `label` is the text alternative. */
export function Sparkline({ values, label }: { values: number[]; label: string }) {
  if (values.length < 2) return <span className="text-xs text-muted-foreground">collecting</span>
  const max = Math.max(...values, 1e-9)
  const w = 80
  const h = 20
  const pts = values
    .map(
      (v, i) => `${((i / (values.length - 1)) * w).toFixed(1)},${(h - (v / max) * h).toFixed(1)}`,
    )
    .join(' ')
  return (
    <svg
      role="img"
      aria-label={label}
      width={w}
      height={h}
      viewBox={`0 0 ${String(w)} ${String(h)}`}
    >
      <polyline points={pts} fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  )
}

export function Bar({ percent, label }: { percent: number | null | undefined; label: string }) {
  const p = Math.max(0, Math.min(100, percent ?? 0))
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(p)}
      className="h-1.5 w-full min-w-12 bg-muted"
    >
      <div className="h-full bg-primary" style={{ width: `${String(p)}%` }} />
    </div>
  )
}

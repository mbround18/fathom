/** Small formatters shared by descriptors (ported from src/app/views/pods.rs). */

/** kubectl-style age: `45s`, `12m`, `5h`, `3d`, `2y`. */
export function formatAge(ts: string | null | undefined, now: number = Date.now()): string {
  if (!ts) return '—'
  const t = Date.parse(ts)
  if (Number.isNaN(t)) return '—'
  const s = Math.max(0, Math.floor((now - t) / 1000))
  if (s < 60) return `${String(s)}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${String(m)}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${String(h)}h`
  const d = Math.floor(h / 24)
  if (d < 365) return `${String(d)}d`
  return `${String(Math.floor(d / 365))}y`
}

export function formatMillicores(m: number | null | undefined): string {
  return m === null || m === undefined ? '—' : `${String(Math.round(m))}m`
}

export function formatMi(bytes: number | null | undefined): string {
  return bytes === null || bytes === undefined ? '—' : `${(bytes / 1024 / 1024).toFixed(0)}Mi`
}

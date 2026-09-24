/** 027 FR-015: `updated Ns ago` label from an ISO8601 timestamp; `null` when unparseable/empty. */
export function formatUpdatedAgo(lastUpdated: string, now: number): string | null {
  const t = Date.parse(lastUpdated)
  if (!lastUpdated || Number.isNaN(t)) return null
  const s = Math.max(0, Math.floor((now - t) / 1000))
  if (s < 60) return `updated ${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `updated ${m}m ago`
  return `updated ${Math.floor(m / 60)}h ago`
}

/**
 * 026-mutation-audit-log-viewer: client-side filtering of the (retention-bounded) audit log.
 * Filter state is transient/session-only: never persisted (spec Key Entities).
 */
import type { AuditActionKind, AuditLogEntry } from '@/protocol/types'

export interface AuditLogFilter {
  actionKinds: AuditActionKind[]
  outcome: 'all' | 'success' | 'failure'
  context: string | null
  /** Inclusive bounds, epoch milliseconds. */
  timeRange: { from?: number; to?: number }
  searchText: string
}

export const DEFAULT_AUDIT_FILTER: AuditLogFilter = {
  actionKinds: [],
  outcome: 'all',
  context: null,
  timeRange: {},
  searchText: '',
}

export function isDefaultFilter(f: AuditLogFilter): boolean {
  return (
    f.actionKinds.length === 0 &&
    f.outcome === 'all' &&
    f.context === null &&
    f.timeRange.from === undefined &&
    f.timeRange.to === undefined &&
    f.searchText.trim() === ''
  )
}

export function applyAuditFilter(
  entries: readonly AuditLogEntry[],
  f: AuditLogFilter,
): AuditLogEntry[] {
  const q = f.searchText.trim().toLowerCase()
  return entries.filter((e) => {
    if (f.actionKinds.length > 0 && !f.actionKinds.includes(e.action)) return false
    if (f.outcome === 'success' && e.outcome.type !== 'Success') return false
    if (f.outcome === 'failure' && e.outcome.type !== 'Failure') return false
    if (f.context !== null && e.context !== f.context) return false
    const t = Date.parse(e.timestamp)
    if (f.timeRange.from !== undefined && !(t >= f.timeRange.from)) return false
    if (f.timeRange.to !== undefined && !(t <= f.timeRange.to)) return false
    if (q !== '' && !(e.targetName ?? '').toLowerCase().includes(q)) return false
    return true
  })
}

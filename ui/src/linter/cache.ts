/**
 * Per-scope linter run cache (feedback 2026-09-27 #24). The linter evaluates cached store rows and
 * details, but its pane used to re-send every list refresh on each mount and forget which detail
 * reads were already in flight, so navigating away and back restarted the whole run (~2000
 * objects on a large cluster). This module outlives the pane and remembers, per context and
 * namespace, the last complete report, when the lists were last refreshed and which reads are out.
 *
 * Change signal: pods and deployments stream through backend watchers into the store, and the
 * report is recomputed from the store whenever those rows change, so those kinds stay current
 * without a re-run. The other sections are one-shot lists and per-object reads with no watch or
 * resourceVersion reaching the UI, so they re-run on a staleness TTL or a manual refresh.
 *
 * Pure bookkeeping: no store access, no commands.
 */
import type { LintReport } from './types'

/** A mount after this long since the last refresh re-lists the linter's sections. */
export const LINT_CACHE_TTL_MS = 5 * 60_000

export interface LintCacheEntry {
  /** Last complete report (every list settled, no pending objects). */
  report?: LintReport
  /** When `report` was produced. */
  evaluatedAt?: number
  /** When the linter last sent its list refreshes for this scope. */
  refreshedAt?: number
  /** Details read before this time count as missing (a manual refresh re-reads them). */
  detailsSince: number
  /** Detail read send times keyed by `detailKeyOf`; kept across mounts so in-flight reads are reused. */
  sent: Map<string, number>
}

const cache = new Map<string, LintCacheEntry>()

export function lintScopeKey(context: string | undefined, namespace: string | null | undefined) {
  return `${context ?? ''}|${namespace ?? '*'}`
}

/** The entry for `key`, created empty on first use. */
export function lintCacheEntry(key: string): LintCacheEntry {
  let entry = cache.get(key)
  if (!entry) {
    entry = { detailsSince: 0, sent: new Map() }
    cache.set(key, entry)
  }
  return entry
}

/**
 * Whether a mount should re-send the list refreshes. A run started within the TTL (finished or
 * still in flight) is reused instead of restarted.
 */
export function needsRefresh(entry: LintCacheEntry, now: number, ttl = LINT_CACHE_TTL_MS) {
  return entry.refreshedAt === undefined || now - entry.refreshedAt > ttl
}

/**
 * Record that the lists are being refreshed now. `rereadDetails` (manual refresh) also marks
 * every cached detail stale and forgets earlier sends, so each object is read again.
 */
export function beginRefresh(entry: LintCacheEntry, now: number, rereadDetails: boolean): void {
  entry.refreshedAt = now
  if (rereadDetails) {
    entry.detailsSince = now
    entry.sent = new Map()
  }
}

/** A detail counts only when it arrived at or after `detailsSince` (see `LintCacheEntry`). */
export function isFresh(detailsSince: number, detailUpdatedAt: number | undefined): boolean {
  return detailUpdatedAt !== undefined && detailUpdatedAt >= detailsSince
}

/** Store `report` if it is complete: every list settled and no object still pending. */
export function recordReport(
  entry: LintCacheEntry,
  report: LintReport,
  listsSettled: boolean,
  now: number,
): boolean {
  if (!listsSettled || report.pendingObjects > 0) return false
  if (entry.report === report) return false
  entry.report = report
  entry.evaluatedAt = now
  return true
}

/**
 * What the pane shows: the live report once it is complete, otherwise the last complete one
 * (so returning to the pane or refreshing never blanks it). With no cached report, the live
 * partial report streams in as before.
 */
export function displayedReport(
  entry: LintCacheEntry,
  live: LintReport,
  listsSettled: boolean,
): LintReport {
  const complete = listsSettled && live.pendingObjects === 0
  return complete || !entry.report ? live : entry.report
}

/** "just now", "42s ago", "3m ago", "2h ago". */
export function formatAge(ms: number): string {
  if (ms < 5_000) return 'just now'
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s ago`
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`
  return `${Math.floor(ms / 3_600_000)}h ago`
}

/** Test hook: forget every scope. */
export function clearLintCache(): void {
  cache.clear()
}

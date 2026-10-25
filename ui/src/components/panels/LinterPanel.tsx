/**
 * Cluster Linter tool pane (017-cluster-linter). Strictly read-only (FR-003): it only sends the
 * existing `Refresh*` list commands and per-object `Select*` reads (through the store's
 * `refreshSection`/`inspect`), and opens the Inspector on click. It never sends a mutation.
 */
import { useCallback, useEffect, useMemo, useState } from 'react'

import { Button } from '@companion/ui'
import {
  beginRefresh,
  displayedReport,
  formatAge,
  isFresh,
  lintCacheEntry,
  lintScopeKey,
  needsRefresh,
  recordReport,
} from '@/linter/cache'
import { detailTargets, nextBatch } from '@/linter/fetchPlan'
import { evaluate } from '@/linter/evaluate'
import { filterFindings } from '@/linter/filter'
import { SECTIONS } from '@/linter/util'
import {
  CATEGORY_LABEL,
  SEVERITIES,
  type Category,
  type Finding,
  type Severity,
} from '@/linter/types'
import { descriptorFor } from '@/resources'
import { useFirstRun } from '@/lib/use-first-run'
import {
  detailKey,
  listKey,
  useStore,
  type DetailState,
  type ListState,
  type PaneId,
} from '@/store'

import { FirstRunState } from '../shell/FirstRunState'
import { Disconnected, Empty } from '@companion/terminal'

const REFRESH_SECTIONS = [
  SECTIONS.pods,
  SECTIONS.deployments,
  SECTIONS.statefulsets,
  SECTIONS.daemonsets,
  SECTIONS.replicasets,
  SECTIONS.configmaps,
  SECTIONS.secrets,
  SECTIONS.pdbs,
  SECTIONS.roles,
  SECTIONS.clusterRoles,
]

const SEVERITY_CLASS: Record<Severity, string> = {
  high: 'text-red-500',
  medium: 'text-amber-500',
  low: 'text-muted-foreground',
}

/** The linter's `LintInput` over the store, keeping the store's arrival timestamps. */
interface StoreInput {
  rows: (section: string) => ListState | undefined
  detail: (kind: string, namespace: string | null, name: string) => DetailState | undefined
}

export function LinterPanel({ paneId }: { paneId: PaneId }) {
  const disconnected = useStore((s) => s.conn.status === 'backend-gone')
  const firstRun = useFirstRun()
  const refreshSection = useStore((s) => s.refreshSection)
  const inspect = useStore((s) => s.inspect)
  const openInspector = useStore((s) => s.openInspector)
  const ctx = useStore((s) => s.session.activeContext)
  const activeNamespace = useStore((s) => s.session.activeNamespace)
  const lists = useStore((s) => s.lists)
  const details = useStore((s) => s.details)

  const [severity, setSeverity] = useState<Severity | null>(null)
  const [category, setCategory] = useState<Category | null>(null)

  const scopeKey = lintScopeKey(ctx, activeNamespace)
  const entry = useMemo(() => lintCacheEntry(scopeKey), [scopeKey])
  // Bumped by a manual refresh so this render picks up the new `entry.detailsSince`.
  const [, setRefreshGen] = useState(0)
  const detailsSince = entry.detailsSince

  // Cached rows/details only: the linter itself performs no cluster access (FR-002). Details
  // older than a manual refresh count as missing, so they are read again.
  const input = useMemo<StoreInput>(
    () => ({
      rows: (section): ListState | undefined => {
        const scope = descriptorFor(section)?.namespaced === false ? 'Cluster' : 'Namespaced'
        return lists[listKey(ctx, section, scope, null)]
      },
      detail: (kind, namespace, name): DetailState | undefined => {
        const d = details[detailKey(ctx, kind, namespace, name)]
        return d && isFresh(detailsSince, d.updatedAt) ? d : undefined
      },
    }),
    [lists, details, ctx, detailsSince],
  )

  const refreshLists = useCallback(() => {
    for (const section of REFRESH_SECTIONS) refreshSection(section)
  }, [refreshSection])

  // Mount / scope change (#24): reuse the cached run for this context and namespace. The lists
  // are re-sent only when this scope has never run, the TTL has passed, or the store lost them.
  useEffect(() => {
    const now = Date.now()
    const { lists: stored } = useStore.getState()
    const missing = REFRESH_SECTIONS.some((section) => {
      const scope = descriptorFor(section)?.namespaced === false ? 'Cluster' : 'Namespaced'
      return stored[listKey(ctx, section, scope, null)] === undefined
    })
    if (!missing && !needsRefresh(entry, now)) return
    beginRefresh(entry, now, false)
    refreshLists()
  }, [refreshLists, ctx, entry])

  const onRefresh = () => {
    beginRefresh(entry, Date.now(), true)
    setRefreshGen((n) => n + 1)
    refreshLists()
  }

  // Throttled, progressive per-object detail reads for the active scope (research R6). The sent
  // map lives in the cache entry, so reads still in flight from an earlier mount are not re-sent.
  const [tick, setTick] = useState(0)
  // `dataAt`: arrival time of the newest list or detail the report was computed from, which is
  // what the age readout reports (how old the result's data is).
  const { live, dataAt } = useMemo(() => {
    let at = 0
    for (const section of REFRESH_SECTIONS) at = Math.max(at, input.rows(section)?.updatedAt ?? 0)
    for (const t of detailTargets(input)) {
      at = Math.max(at, input.detail(t.kind, t.namespace, t.name)?.updatedAt ?? 0)
    }
    return { live: evaluate(input), dataAt: at }
  }, [input])
  useEffect(() => {
    const now = Date.now()
    const batch = nextBatch(
      detailTargets(input),
      (t) => input.detail(t.kind, t.namespace, t.name) !== undefined,
      entry.sent,
      now,
    )
    for (const t of batch) {
      entry.sent.set(`${t.kind}|${t.namespace ?? '*'}|${t.name}`, now)
      inspect(t.kind, t.namespace ?? '', t.name)
    }
  }, [input, entry, inspect, tick])
  // Re-run while reads are outstanding so expired requests free their slots.
  useEffect(() => {
    if (live.pendingObjects === 0) return
    const id = setInterval(() => setTick((n) => n + 1), 2000)
    return () => clearInterval(id)
  }, [live.pendingObjects])

  // A section is settled once it is not loading (ready, failed or denied all count).
  const listsSettled = REFRESH_SECTIONS.every((s) => input.rows(s)?.status !== 'loading')
  // Cache bookkeeping only (idempotent): remember the report once the run is complete.
  const evaluatedAt = useMemo(() => {
    recordReport(entry, live, listsSettled, dataAt)
    return entry.evaluatedAt
  }, [entry, live, listsSettled, dataAt])
  const report = displayedReport(entry, live, listsSettled)

  // Re-render the age readout now and then.
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 15_000)
    return () => clearInterval(id)
  }, [])

  const visible = useMemo(
    () => filterFindings(report.findings, { severity, category }),
    [report.findings, severity, category],
  )
  const skipped = report.rules.filter((r) => r.status === 'skipped' || r.reason !== undefined)

  if (disconnected) return <Disconnected />
  if (firstRun) return <FirstRunState state={firstRun} />

  return (
    <div className="flex h-full min-h-0 w-full min-w-0 flex-col" data-testid="linter-panel">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-xs">
        <span data-testid="linter-score" className="font-medium">
          Score: {report.score === null ? '-' : `${report.score}%`}
        </span>
        {SEVERITIES.map((s) => (
          <Button
            key={s}
            size="sm"
            variant={severity === s ? 'secondary' : 'ghost'}
            data-testid={`linter-severity-${s}`}
            aria-pressed={severity === s}
            onClick={() => {
              setSeverity(severity === s ? null : s)
            }}
          >
            <span className={SEVERITY_CLASS[s]}>{s}</span> {report.counts[s]}
          </Button>
        ))}
        <select
          aria-label="Category"
          data-testid="linter-category"
          className="h-7 rounded border border-border bg-background px-1"
          value={category ?? ''}
          onChange={(e) => {
            setCategory(e.target.value === '' ? null : (e.target.value as Category))
          }}
        >
          <option value="">All categories</option>
          {(Object.keys(CATEGORY_LABEL) as Category[]).map((c) => (
            <option key={c} value={c}>
              {CATEGORY_LABEL[c]}
            </option>
          ))}
        </select>
        {(severity !== null || category !== null) && (
          <Button
            size="sm"
            variant="ghost"
            data-testid="linter-clear-filters"
            onClick={() => {
              setSeverity(null)
              setCategory(null)
            }}
          >
            Clear
          </Button>
        )}
        <span className="ml-auto flex items-center gap-2">
          {evaluatedAt !== undefined && (
            <span data-testid="linter-age" className="text-muted-foreground">
              Updated {formatAge(Math.max(0, now - evaluatedAt))}
            </span>
          )}
          <Button size="sm" variant="ghost" data-testid="linter-refresh" onClick={onRefresh}>
            Refresh
          </Button>
        </span>
      </div>

      {(live.pendingObjects > 0 || !listsSettled) && (
        <div
          className="border-b border-border px-3 py-1 text-xs text-muted-foreground"
          data-testid="linter-progress"
        >
          {live.pendingObjects > 0
            ? `Still evaluating ${live.pendingObjects} objects...`
            : 'Refreshing resource lists...'}
          {report !== live && ' Showing the last result until this run finishes.'}
        </div>
      )}
      {skipped.length > 0 && (
        <ul
          className="border-b border-border px-3 py-1 text-xs text-muted-foreground"
          data-testid="linter-skipped"
        >
          {skipped.map((r) => (
            <li key={r.rule}>
              {r.status === 'skipped' ? 'Skipped' : 'Partial'}: {r.title} - {r.reason}
            </li>
          ))}
        </ul>
      )}

      {visible.length === 0 ? (
        <Empty
          title={report.findings.length === 0 ? 'No findings' : 'No findings match the filters'}
          detail={
            live.pendingObjects > 0 || !listsSettled
              ? 'Evaluation is still in progress.'
              : 'Every checked resource passed the enabled rules.'
          }
        />
      ) : (
        <ul className="min-h-0 flex-1 overflow-auto" data-testid="linter-findings">
          {visible.map((f) => (
            <FindingRow
              key={f.id}
              finding={f}
              onOpen={() => {
                openInspector(f.ref, paneId)
              }}
            />
          ))}
        </ul>
      )}
    </div>
  )
}

function FindingRow({ finding, onOpen }: { finding: Finding; onOpen: () => void }) {
  const where = finding.ref.namespace
    ? `${finding.ref.namespace}/${finding.ref.name}`
    : finding.ref.name
  return (
    <li>
      <button
        type="button"
        data-testid="linter-finding"
        data-rule={finding.rule}
        className="flex w-full flex-col gap-0.5 border-b border-border px-3 py-2 text-left text-sm hover:bg-accent"
        onClick={onOpen}
      >
        <span className="flex items-center gap-2">
          <span className={`text-xs uppercase ${SEVERITY_CLASS[finding.severity]}`}>
            {finding.severity}
          </span>
          <span className="truncate font-medium">
            {finding.ref.kind} {where}
          </span>
        </span>
        <span className="text-xs text-muted-foreground">{finding.message}</span>
      </button>
    </li>
  )
}

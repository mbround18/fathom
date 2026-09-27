/**
 * Cluster Linter tool pane (017-cluster-linter). Strictly read-only (FR-003): it only sends the
 * existing `Refresh*` list commands and per-object `Select*` reads (through the store's
 * `refreshSection`/`inspect`), and opens the Inspector on click. It never sends a mutation.
 */
import { useEffect, useMemo, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { detailTargets, nextBatch } from '@/linter/fetchPlan'
import { evaluate } from '@/linter/evaluate'
import { filterFindings } from '@/linter/filter'
import { SECTIONS } from '@/linter/util'
import {
  CATEGORY_LABEL,
  SEVERITIES,
  type Category,
  type Finding,
  type LintInput,
  type Severity,
} from '@/linter/types'
import { descriptorFor } from '@/resources'
import { useFirstRun } from '@/lib/use-first-run'
import { detailKey, listKey, useStore, type PaneId } from '@/store'

import { FirstRunState } from '../shell/FirstRunState'
import { Disconnected, Empty } from '../shell/PaneStates'

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

  // Cached rows/details only: the linter itself performs no cluster access (FR-002).
  const input = useMemo<LintInput>(
    () => ({
      rows: (section) => {
        const scope = descriptorFor(section)?.namespaced === false ? 'Cluster' : 'Namespaced'
        return lists[listKey(ctx, section, scope, null)]
      },
      detail: (kind, namespace, name) => details[detailKey(ctx, kind, namespace, name)],
    }),
    [lists, details, ctx],
  )

  // Mount / scope change: refresh the lists the rules read (existing commands only).
  useEffect(() => {
    for (const section of REFRESH_SECTIONS) refreshSection(section)
  }, [refreshSection, ctx, activeNamespace])

  // Throttled, progressive per-object detail reads for the active scope (research R6).
  const sent = useRef(new Map<string, number>())
  const sentCtx = useRef(ctx)
  const [tick, setTick] = useState(0)
  const report = useMemo(() => evaluate(input), [input])
  useEffect(() => {
    if (sentCtx.current !== ctx) {
      sentCtx.current = ctx
      sent.current = new Map()
    }
    const now = Date.now()
    const batch = nextBatch(
      detailTargets(input),
      (t) => input.detail(t.kind, t.namespace, t.name) !== undefined,
      sent.current,
      now,
    )
    for (const t of batch) {
      sent.current.set(`${t.kind}|${t.namespace ?? '*'}|${t.name}`, now)
      inspect(t.kind, t.namespace ?? '', t.name)
    }
  }, [input, ctx, inspect, tick])
  // Re-run while reads are outstanding so expired requests free their slots.
  useEffect(() => {
    if (report.pendingObjects === 0) return
    const id = setInterval(() => setTick((n) => n + 1), 2000)
    return () => clearInterval(id)
  }, [report.pendingObjects])

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
      </div>

      {report.pendingObjects > 0 && (
        <div
          className="border-b border-border px-3 py-1 text-xs text-muted-foreground"
          data-testid="linter-progress"
        >
          Still evaluating {report.pendingObjects} objects...
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
            report.pendingObjects > 0
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

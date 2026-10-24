/**
 * `/admin/audit` (US5, T070, T074): the audit log, newest first as the server pages it. One
 * virtualised scroller, so a long log stays cheap; each entry is a table row on wide screens and a
 * card on narrow ones (the same element, laid out by CSS). The next page loads as the end scrolls
 * into view.
 *
 * Read-only by design (FR-036): no control here edits or removes an entry. Export is a plain
 * same-origin download of `GET /api/v1/audit/export` with the current filter, as JSONL.
 */
import { ArrowClockwiseIcon, DownloadSimpleIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useMemo, useRef } from 'react'

import { formatTime } from '@/features/stacks/model'
import { useStacks } from '@/features/stacks/store'
import type { AuditEntry } from '@/protocol/generated'

import { AuditFilters } from './AuditFilters'
import { ACTION_LABEL, actorId, detailLines, exportHref, outcomeText } from './model'
import { useAudit } from './store'

const ESTIMATED_ROW_PX = 64
const OVERSCAN = 8
/** Ask for the next page when this many rows are left below the viewport. */
const PREFETCH_ROWS = 10
const NO_NAMES: readonly string[] = []

/** Wide: one grid row per entry; narrow: a card, each cell with its own label. */
const ROW = 'grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-0.5 px-2 py-1.5 text-xs md:grid-cols-[8.5rem_9rem_7.5rem_7rem_minmax(0,1fr)_8rem] md:gap-y-0'
const LABEL = 'text-muted-foreground md:hidden'

export default function AuditLog() {
  const filter = useAudit((s) => s.filter)
  const entries = useAudit((s) => s.entries)
  const nextCursor = useAudit((s) => s.nextCursor)
  const loading = useAudit((s) => s.loading)
  const loaded = useAudit((s) => s.loaded)
  const denied = useAudit((s) => s.denied)
  const setFilter = useAudit((s) => s.setFilter)
  const refresh = useAudit((s) => s.refresh)
  const more = useAudit((s) => s.more)
  const loadStacks = useStacks((s) => s.load)
  const stackList = useStacks((s) => s.list)
  const stackNames = useMemo(() => stackList?.stacks.map((s) => s.name) ?? NO_NAMES, [stackList])

  useEffect(() => {
    refresh()
    loadStacks()
  }, [refresh, loadStacks])

  const scrollRef = useRef<HTMLDivElement>(null)
  // The app does not run the React Compiler; the virtualizer is read fresh every render.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_PX,
    overscan: OVERSCAN,
  })
  const virtual = virtualizer.getVirtualItems()
  const first = virtual[0]
  const last = virtual[virtual.length - 1]
  const total = virtualizer.getTotalSize()
  const lastIndex = last?.index ?? -1

  useEffect(() => {
    if (nextCursor !== null && !loading && lastIndex >= entries.length - PREFETCH_ROWS) more()
  }, [lastIndex, entries.length, nextCursor, loading, more])

  return (
    <section data-page="audit" aria-labelledby="audit-title" className="flex h-full min-w-0 flex-col">
      <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
        <h1 id="audit-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Audit log
        </h1>
        <span data-testid="audit-count" className="text-xs text-muted-foreground tabular-nums">
          {loaded ? `${String(entries.length)}${nextCursor !== null ? '+' : ''} entries` : ''}
        </span>
        <span className="flex-1" />
        <Button variant="ghost" size="sm" onClick={refresh} aria-label="Refresh the audit log">
          <ArrowClockwiseIcon aria-hidden />
        </Button>
        <a
          href={exportHref(filter)}
          download="depths-audit.jsonl"
          data-action="export"
          className="inline-flex h-7 shrink-0 items-center gap-1 border border-input px-2 text-xs hover:border-primary hover:text-primary"
        >
          <DownloadSimpleIcon aria-hidden className="size-3.5" />
          Export JSONL
        </a>
      </div>

      <AuditFilters filter={filter} stacks={stackNames} onChange={setFilter} />

      <div
        ref={scrollRef}
        role="table"
        aria-label="Audit entries"
        aria-rowcount={entries.length + 1}
        data-testid="audit-scroller"
        className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto"
      >
        <div role="rowgroup" className="sticky top-0 z-10 hidden border-b border-border bg-background md:block">
          <div role="row" aria-rowindex={1} className={`${ROW} font-semibold text-muted-foreground`}>
            <span role="columnheader">Time</span>
            <span role="columnheader">Actor</span>
            <span role="columnheader">Action</span>
            <span role="columnheader">Stack</span>
            <span role="columnheader">Details</span>
            <span role="columnheader">Outcome</span>
          </div>
        </div>

        {denied ? (
          <p role="status" className="p-3 text-xs text-muted-foreground">
            You can&apos;t read the audit log.
          </p>
        ) : !loaded ? (
          <p role="status" data-pane-state="loading" className="p-3 text-xs text-muted-foreground">
            Loading the audit log…
          </p>
        ) : entries.length === 0 ? (
          <p role="status" data-pane-state="empty" className="p-3 text-xs text-muted-foreground">
            No entries match the filter.
          </p>
        ) : (
          <div role="rowgroup" style={{ paddingTop: first?.start ?? 0, paddingBottom: last ? total - last.end : 0 }}>
            {virtual.map((v) => {
              const entry = entries[v.index]
              if (!entry) return null
              return (
                <div key={v.key} data-index={v.index} ref={virtualizer.measureElement}>
                  <Row entry={entry} index={v.index} />
                </div>
              )
            })}
          </div>
        )}
        {loading && loaded && (
          <p role="status" className="p-2 text-xs text-muted-foreground">
            Loading more…
          </p>
        )}
      </div>
    </section>
  )
}

function Row({ entry, index }: { entry: AuditEntry; index: number }) {
  const outcome = entry.outcome.kind
  return (
    <div
      role="row"
      aria-rowindex={index + 2}
      data-audit-row
      data-action={entry.action}
      data-outcome={outcome}
      className={`${ROW} border-b border-border`}
    >
      <span className={LABEL}>Time</span>
      <span role="cell" className="tabular-nums">
        <time dateTime={entry.at}>{formatTime(entry.at)}</time>
      </span>
      <span className={LABEL}>Actor</span>
      <span role="cell" className="min-w-0 truncate" title={actorId(entry)}>
        {entry.actor.username} <span className="text-muted-foreground">({entry.actor.provider})</span>
      </span>
      <span className={LABEL}>Action</span>
      <span role="cell">{ACTION_LABEL[entry.action]}</span>
      <span className={LABEL}>Stack</span>
      <span role="cell" className="min-w-0 truncate">
        {entry.stack ?? '—'}
      </span>
      <span className={LABEL}>Details</span>
      <span role="cell" className="min-w-0 font-mono break-words whitespace-pre-wrap">
        {detailLines(entry).join('\n') || '—'}
      </span>
      <span className={LABEL}>Outcome</span>
      <span
        role="cell"
        className={outcome === 'succeeded' ? 'text-status-ok' : outcome === 'failed' ? 'text-status-err' : 'text-status-warn'}
      >
        {outcomeText(entry.outcome)}
      </span>
    </div>
  )
}

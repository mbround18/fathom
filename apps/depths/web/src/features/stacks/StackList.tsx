/**
 * The stack list (US1, T049): every stack the session may see, then, for admins, foreign projects
 * and standalone containers, read-only. One virtualised scroller holds all three groups, so 200
 * stacks and their neighbours stay cheap to render and to filter (SC-004).
 */
import { MagnifyingGlassIcon, PlusIcon, WarningOctagonIcon } from '@phosphor-icons/react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useMemo, useRef } from 'react'

import { Link } from '@/app/Link'
import { useSession } from '@/app/session-store'
import type { Container, ForeignProject, Stack } from '@/protocol/generated'

import { ForeignRow, GroupHeading, StandaloneRow } from './ForeignList'
import {
  elsewhereByStack,
  filterByName,
  filterStacks,
  formatTime,
  STATUS_LABEL,
  STATUS_ORDER,
  type StatusFilter,
} from './model'
import { StackRow } from './StackRow'
import { useStacks } from './store'

type Item =
  | { key: string; kind: 'stack'; stack: Stack }
  | { key: string; kind: 'heading'; id: string; label: string; count: number }
  | { key: string; kind: 'foreign'; project: ForeignProject }
  | { key: string; kind: 'standalone'; container: Container }

const NO_ELSEWHERE: readonly ForeignProject[] = []
const ESTIMATED_ROW_PX = 56
const OVERSCAN = 10

export function StackList() {
  const list = useStacks((s) => s.list)
  const runtimeError = useStacks((s) => s.runtimeError)
  const listDenied = useStacks((s) => s.listDenied)
  const query = useStacks((s) => s.query)
  const status = useStacks((s) => s.status)
  const setQuery = useStacks((s) => s.setQuery)
  const setStatus = useStacks((s) => s.setStatus)
  const load = useStacks((s) => s.load)

  useEffect(() => {
    load()
  }, [load])

  const elsewhere = useMemo(() => elsewhereByStack(list?.foreign ?? []), [list])
  const shownStacks = useMemo(() => filterStacks(list?.stacks ?? [], query, status), [list, query, status])
  const items = useMemo((): Item[] => {
    const out: Item[] = shownStacks.map((stack) => ({ key: `s:${stack.name}`, kind: 'stack', stack }))
    const foreign = filterByName(list?.foreign ?? [], query, status)
    if (foreign.length > 0) {
      out.push({ key: 'h:foreign', kind: 'heading', id: 'group-foreign', label: 'Not in the stacks directory', count: foreign.length })
      for (const project of foreign) out.push({ key: `f:${project.name}`, kind: 'foreign', project })
    }
    const standalone = filterByName(list?.standalone ?? [], query, status)
    if (standalone.length > 0) {
      out.push({ key: 'h:standalone', kind: 'heading', id: 'group-standalone', label: 'Standalone containers', count: standalone.length })
      for (const container of standalone) out.push({ key: `c:${container.id}`, kind: 'standalone', container })
    }
    return out
  }, [shownStacks, list, query, status])

  const scrollRef = useRef<HTMLDivElement>(null)
  // The app does not run the React Compiler; the virtualizer is read fresh every render.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_PX,
    getItemKey: (i) => items[i]?.key ?? i,
    overscan: OVERSCAN,
  })
  const virtual = virtualizer.getVirtualItems()
  const total = virtualizer.getTotalSize()
  const first = virtual[0]
  const last = virtual[virtual.length - 1]

  const totalStacks = list?.stacks.length ?? 0

  return (
    <section data-page="stacks" aria-labelledby="stacks-title" className="flex h-full min-w-0 flex-col">
      <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
        <h1 id="stacks-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Stacks
        </h1>
        <span data-testid="stack-count" className="text-xs text-muted-foreground tabular-nums">
          {list ? `${String(shownStacks.length)} of ${String(totalStacks)}` : ''}
        </span>
        <span className="flex-1" />
        <NewStackLink />
        <label className="flex h-7 min-w-0 flex-1 basis-40 items-center gap-1 border border-input bg-background px-1.5 focus-within:border-ring">
          <MagnifyingGlassIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
          <input
            type="search"
            aria-label="Filter by name"
            placeholder="Filter by name"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="h-full min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
          />
        </label>
        <select
          aria-label="Filter by status"
          value={status}
          onChange={(e) => setStatus(e.target.value as StatusFilter)}
          className="h-7 shrink-0 border border-input bg-background px-1 text-xs focus-visible:border-ring"
        >
          <option value="all">All statuses</option>
          {STATUS_ORDER.map((s) => (
            <option key={s} value={s}>
              {STATUS_LABEL[s]}
            </option>
          ))}
        </select>
      </div>

      {runtimeError !== null && (
        <div role="alert" data-banner="runtime" className="flex shrink-0 items-start gap-1.5 border-b border-status-err/50 px-2 py-1.5 text-xs text-status-err">
          <WarningOctagonIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          <p className="min-w-0 break-words">
            The container runtime is unreachable: <span className="whitespace-pre-wrap">{runtimeError}</span>
            {list && <> Showing the last known state, as of {formatTime(list.as_of)}.</>}
          </p>
        </div>
      )}
      {list && list.warnings.length > 0 && (
        <ul data-banner="warnings" className="flex shrink-0 flex-col gap-0.5 border-b border-status-warn/50 px-2 py-1.5 text-xs text-status-warn">
          {list.warnings.map((w, i) => (
            <li key={i} className="min-w-0 break-words whitespace-pre-wrap">
              {w}
            </li>
          ))}
        </ul>
      )}

      <div
        ref={scrollRef}
        data-testid="stack-scroller"
        data-stale={list?.stale || undefined}
        className={`min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto ${list?.stale ? 'opacity-70' : ''}`}
      >
        {listDenied ? (
          <p role="status" className="p-3 text-xs text-muted-foreground">
            You can&apos;t list stacks.
          </p>
        ) : !list ? (
          <p role="status" data-pane-state="loading" className="p-3 text-xs text-muted-foreground">
            Loading stacks…
          </p>
        ) : items.length === 0 ? (
          <p role="status" data-pane-state="empty" className="p-3 text-xs break-words text-muted-foreground">
            {totalStacks === 0 ? `No stacks in ${list.root}.` : 'No stacks match the filter.'}
          </p>
        ) : (
          <div style={{ paddingTop: first?.start ?? 0, paddingBottom: last ? total - last.end : 0 }}>
            {virtual.map((v) => {
              const item = items[v.index]
              if (!item) return null
              return (
                <div key={v.key} data-index={v.index} ref={virtualizer.measureElement}>
                  {item.kind === 'stack' ? (
                    <StackRow stack={item.stack} elsewhere={elsewhere.get(item.stack.name) ?? NO_ELSEWHERE} />
                  ) : item.kind === 'heading' ? (
                    <GroupHeading id={item.id} label={item.label} count={item.count} />
                  ) : item.kind === 'foreign' ? (
                    <ForeignRow project={item.project} />
                  ) : (
                    <StandaloneRow container={item.container} />
                  )}
                </div>
              )
            })}
          </div>
        )}
      </div>
    </section>
  )
}

/** Creating a stack is for admins, and only while editing is on (FR-022). */
function NewStackLink() {
  const show = useSession((s) => s.me?.role === 'admin' && s.me.features.edit)
  if (!show) return null
  return (
    <Link
      to="/new"
      data-action="new-stack"
      className="inline-flex h-7 shrink-0 items-center gap-1 border border-input px-2 text-xs hover:border-primary hover:text-primary"
    >
      <PlusIcon aria-hidden className="size-3.5" />
      New stack
    </Link>
  )
}

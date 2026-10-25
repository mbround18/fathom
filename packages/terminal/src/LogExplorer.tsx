/**
 * A log view with search, level facets and "show in context", shared by every log pane.
 *
 * The search box takes the `log-query` language (free text, "phrases", -exclusions, level:…,
 * key:value). Level chips count the lines the query leaves, per level, and toggle a level filter.
 * While anything narrows the view, each result row is a button: activating it switches to the
 * whole log (context), scrolls to that exact line and flashes it, and pauses follow so the line
 * stays put. "Back to results" returns to the narrowed view.
 *
 * The caller keeps its own toolbar (follow, pause, wrap, …) and passes it as `toolbarStart` /
 * `toolbarEnd`; it can add its own narrowing (a time window) with `filter`.
 */
import { ArrowUUpLeftIcon } from '@phosphor-icons/react'
import { useCallback, useDeferredValue, useMemo, useState, type ReactNode } from 'react'

import { cn } from '@companion/theme'
import { Button, Input } from '@companion/ui'

import { LOG_LEVELS, sharedLineCache, type LineCache, type LogLevel } from './log-parse'
import { filterLines, parseQuery } from './log-query'
import { LogView, type LogFocus, type LogViewProps } from './LogView'

/** Dot colour per level chip; opaque theme tokens. */
const LEVEL_DOT: Record<LogLevel, string> = {
  fatal: 'bg-status-err',
  error: 'bg-status-err',
  warn: 'bg-status-warn',
  info: 'bg-accent-neon',
  debug: 'bg-muted-foreground',
  trace: 'bg-muted-foreground',
}

export const LOG_QUERY_HINT = 'Search: text "phrase" -exclude level:error key:value'

export interface LogExplorerProps
  extends Omit<LogViewProps, 'rows' | 'highlights' | 'onActivateLine' | 'focus' | 'cache'> {
  /** Controls before the search box (follow, pause, wrap, …). */
  toolbarStart?: ReactNode
  /** Controls after the facets (a time window, a "new lines" count, …). */
  toolbarEnd?: ReactNode
  /** Shown between the bar and the lines (a "stream ended" notice, …). */
  banner?: ReactNode
  /** The caller's own narrowing, by index into `lines`; composes with the query. */
  filter?: (index: number) => boolean
  /** Whether `filter` currently narrows anything (it is skipped when false). */
  filterActive?: boolean
  /** Accessible name of the search box. */
  searchLabel?: string
  barClassName?: string
  /** Shown when nothing matches. */
  noMatch?: ReactNode
  cache?: LineCache
}

const DEFAULT_NO_MATCH = (
  <p role="status" data-pane-state="no-match" className="text-xs text-muted-foreground">
    No line matches.
  </p>
)

export function LogExplorer({
  lines,
  follow,
  onFollowChange,
  placeholder,
  toolbarStart,
  toolbarEnd,
  banner,
  filter,
  filterActive = false,
  searchLabel = 'Search logs',
  barClassName,
  noMatch = DEFAULT_NO_MATCH,
  cache = sharedLineCache,
  ...view
}: LogExplorerProps) {
  const [input, setInput] = useState('')
  const [facet, setFacet] = useState<ReadonlySet<LogLevel>>(() => new Set())
  const [context, setContext] = useState<number | null>(null)
  const [focus, setFocus] = useState<LogFocus | null>(null)
  const deferred = useDeferredValue(input)

  const query = useMemo(() => parseQuery(deferred), [deferred])
  const narrowing = !query.empty || facet.size > 0 || filterActive
  const result = useMemo(
    () => filterLines(lines, query, { facet, extra: filterActive ? filter : undefined, cache }),
    [lines, query, facet, filter, filterActive, cache],
  )
  const inContext = context !== null && narrowing

  const jump = useCallback(
    (line: number) => {
      setContext(line)
      setFocus((f) => ({ line, seq: (f?.seq ?? 0) + 1 }))
      onFollowChange?.(false)
    },
    [onFollowChange],
  )

  const back = () => {
    if (context !== null) setFocus((f) => ({ line: context, seq: (f?.seq ?? 0) + 1 }))
    setContext(null)
  }

  const toggleLevel = (level: LogLevel) => {
    setContext(null)
    setFacet((prev) => {
      const next = new Set(prev)
      if (next.has(level)) next.delete(level)
      else next.add(level)
      return next
    })
  }

  const shownPlaceholder =
    placeholder ?? (narrowing && !inContext && result.indices.length === 0 ? noMatch : undefined)

  return (
    <>
      <div
        data-log-bar
        className={cn(
          'flex min-w-0 shrink-0 flex-wrap items-center gap-1 border-b border-border px-1 py-0.5 text-xs',
          barClassName,
        )}
      >
        {toolbarStart}
        <Input
          type="search"
          aria-label={searchLabel}
          placeholder="Search"
          title={LOG_QUERY_HINT}
          value={input}
          onChange={(e) => {
            setInput(e.target.value)
            setContext(null)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape' && input !== '') {
              e.preventDefault()
              setInput('')
              setContext(null)
            }
          }}
          className="h-6 w-40 min-w-0 flex-1 text-xs sm:flex-none"
        />
        <div role="group" aria-label="Levels" className="flex flex-wrap items-center gap-0.5">
          {LOG_LEVELS.filter((l) => result.counts[l] > 0 || facet.has(l)).map((level) => {
            const on = facet.has(level)
            return (
              <Button
                key={level}
                type="button"
                variant={on ? 'outline' : 'ghost'}
                size="xs"
                aria-pressed={on}
                aria-label={`Level ${level}, ${String(result.counts[level])} lines`}
                data-level-facet={level}
                onClick={() => toggleLevel(level)}
                className="px-1.5"
              >
                <span aria-hidden className={cn('size-2 shrink-0', LEVEL_DOT[level])} />
                <span aria-hidden>{level}</span>
                <span aria-hidden className="text-muted-foreground tabular-nums">
                  {result.counts[level]}
                </span>
              </Button>
            )
          })}
        </div>
        {narrowing && !inContext && (
          <span data-field="matches" className="px-1 text-muted-foreground">
            {result.indices.length} of {lines.length}
          </span>
        )}
        {inContext && (
          <Button type="button" variant="outline" size="xs" data-action="back-to-results" onClick={back}>
            <ArrowUUpLeftIcon data-icon="inline-start" aria-hidden /> Back to results
          </Button>
        )}
        {toolbarEnd}
      </div>
      {banner}
      <LogView
        lines={lines}
        follow={follow}
        onFollowChange={onFollowChange}
        placeholder={shownPlaceholder}
        rows={narrowing && !inContext ? result.indices : undefined}
        highlights={query.highlights}
        onActivateLine={narrowing && !inContext ? jump : undefined}
        focus={focus}
        cache={cache}
        data-view={inContext ? 'context' : narrowing ? 'results' : 'all'}
        {...view}
      />
    </>
  )
}

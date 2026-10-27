/**
 * A scrolling, virtualised view over a log's lines.
 *
 * It does the three things every log pane needs, whatever the transport: it pins the newest line
 * while `follow` is on, it reports when the operator scrolls away (or back), and it says when
 * older lines were dropped. Status, empty and error states belong to the caller, which passes
 * one as `placeholder`.
 *
 * Lines arrive raw. Each is parsed once (cached): ANSI SGR colour becomes styled runs, every
 * other control sequence is removed, and a level is detected for the gutter. What reaches the DOM
 * is React text only, never markup from the log.
 *
 * With `rows` the view shows a subset (search results) by index into `lines`; `onActivateLine`
 * makes each row a button ("view in context"); `focus` scrolls to a line and flashes it.
 *
 * Only the visible window is in the DOM once a log is long (`FULL_RENDER_MAX`); a short log is
 * rendered whole, without the virtualiser, so its every line stays findable with the browser's own
 * search and is there before the first layout.
 */
import { BracketsCurlyIcon } from '@phosphor-icons/react'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  memo,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react'

import { cn } from '@companion/theme'

import { prettyJson, sharedLineCache, type LineCache, type LogLevel } from './log-parse'
import { renderLineText } from './log-render'

/** How close to the bottom (px) still counts as "at the bottom" for follow. */
export const FOLLOW_THRESHOLD_PX = 24
/** Up to this many lines are all rendered; past it, only the visible window and an overscan. */
export const FULL_RENDER_MAX = 1_000
/** How long a jumped-to line stays highlighted. */
export const FLASH_MS = 1_600
const OVERSCAN = 20
const ESTIMATED_ROW_PX = 20

export type DroppedCount = number | boolean

/** Ask the view to scroll to `line` (an index into `lines`) and flash it; bump `seq` to repeat. */
export interface LogFocus {
  line: number
  seq: number
}

export interface LogViewProps extends Omit<HTMLAttributes<HTMLDivElement>, 'onScroll'> {
  /** Raw lines; ANSI colour is rendered, other control sequences are removed. */
  lines: readonly string[]
  /** Keep the newest line in view as lines arrive. */
  follow: boolean
  /** Wrap long lines; otherwise they scroll sideways. */
  wrap: boolean
  /** Called when the operator scrolls to the very top, to fetch earlier lines. */
  onLoadOlder?: () => void
  /** The operator scrolled away from the bottom (`false`) or back to it (`true`). */
  onFollowChange?: (follow: boolean) => void
  /** Older lines dropped before the first one shown: a count, or `true` when it is unknown. */
  dropped?: DroppedCount
  /** Wording for the dropped marker. */
  droppedText?: (dropped: number | true) => string
  /** Shown instead of the lines, e.g. a loading or empty state. */
  placeholder?: ReactNode
  /** Show only these lines (indices into `lines`, ascending). All lines when absent. */
  rows?: readonly number[]
  /** Lower-cased substrings to mark in each line. */
  highlights?: readonly string[]
  /** Makes each row activatable (click, Enter, Space): "show this line in context". */
  onActivateLine?: (line: number) => void
  /** Scroll to a line and flash it. */
  focus?: LogFocus | null
  /** Parsed-line cache; the shared one by default. */
  cache?: LineCache
}

export function defaultDroppedText(dropped: number | true): string {
  if (dropped === true) return 'Older lines dropped'
  return `${String(dropped)} older line${dropped === 1 ? '' : 's'} dropped`
}

/** Opaque gutter colours per level (theme tokens; no alpha, so contrast holds in both themes). */
const LEVEL_GUTTER: Record<LogLevel | 'none', string> = {
  fatal: 'border-status-err',
  error: 'border-status-err',
  warn: 'border-status-warn',
  info: 'border-accent-neon',
  debug: 'border-muted-foreground',
  trace: 'border-muted-foreground',
  none: 'border-transparent',
}

const NO_HIGHLIGHTS: readonly string[] = []

interface RowProps {
  raw: string
  cache: LineCache
  position: number
  line: number
  last: boolean
  highlights: readonly string[]
  activatable: boolean
  onActivate?: (line: number) => void
  expanded: boolean
  onToggle: (line: number) => void
  flash: boolean
  measure?: (el: Element | null) => void
}

function selectionInside(el: Element): boolean {
  const sel = typeof window === 'undefined' ? null : window.getSelection()
  if (!sel || sel.isCollapsed || sel.toString() === '') return false
  return sel.anchorNode !== null && el.contains(sel.anchorNode)
}

const Row = memo(function Row({
  raw,
  cache,
  position,
  line,
  last,
  highlights,
  activatable,
  onActivate,
  expanded,
  onToggle,
  flash,
  measure,
}: RowProps) {
  const parsed = cache.get(raw)
  const text = renderLineText(parsed, highlights)
  const activate = (e: MouseEvent<HTMLSpanElement>) => {
    // Selecting text to copy it is not a request to jump.
    if (selectionInside(e.currentTarget)) return
    onActivate?.(line)
  }
  const onKey = (e: KeyboardEvent<HTMLSpanElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onActivate?.(line)
    }
  }
  return (
    <div
      ref={measure}
      data-index={position}
      data-line={line}
      data-level={parsed.level ?? undefined}
      data-flash={flash ? '' : undefined}
      tabIndex={-1}
      className={cn(
        'min-h-[1lh] border-l-2 pl-1.5 transition-colors duration-700 outline-none',
        LEVEL_GUTTER[parsed.level ?? 'none'],
        flash && 'bg-accent-neon/20',
      )}
    >
      {parsed.json && (
        <button
          type="button"
          aria-label={expanded ? 'Collapse JSON' : 'Expand JSON'}
          aria-expanded={expanded}
          title={expanded ? 'Collapse JSON' : 'Expand JSON'}
          onClick={() => onToggle(line)}
          className={cn(
            'mr-1 inline-flex size-[1lh] translate-y-0.5 items-center justify-center align-top text-muted-foreground select-none hover:text-foreground focus-visible:outline-1 focus-visible:outline-ring',
            expanded && 'text-accent-neon',
          )}
        >
          <BracketsCurlyIcon aria-hidden className="size-3" />
        </button>
      )}
      {activatable ? (
        <span
          role="button"
          tabIndex={0}
          title="Show in context"
          data-action="show-in-context"
          onClick={activate}
          onKeyDown={onKey}
          className="cursor-pointer hover:bg-muted focus-visible:outline-1 focus-visible:outline-ring"
        >
          {text}
        </span>
      ) : (
        text
      )}
      {expanded && parsed.json ? (
        <pre
          data-json-pretty
          className="my-0.5 border-l border-border pl-2 break-words whitespace-pre-wrap text-foreground"
        >
          {prettyJson(parsed.json)}
        </pre>
      ) : null}
      {last ? null : '\n'}
    </div>
  )
})

export function LogView({
  lines,
  follow,
  wrap,
  onLoadOlder,
  onFollowChange,
  dropped = 0,
  droppedText = defaultDroppedText,
  placeholder,
  rows,
  highlights = NO_HIGHLIGHTS,
  onActivateLine,
  focus,
  cache = sharedLineCache,
  className,
  ...rest
}: LogViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const atTopRef = useRef(false)
  // Expansion and flash are keyed by absolute line number (dropped + index), so they stay on the
  // same line while the ring drops older ones.
  const base = typeof dropped === 'number' ? dropped : 0
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set())
  const [flashAbs, setFlashAbs] = useState<number | null>(null)

  const count = rows ? rows.length : lines.length
  const lineAt = (position: number) => (rows ? (rows[position] ?? -1) : position)

  const whole = count <= FULL_RENDER_MAX
  const virtualizer = useVirtualizer({
    enabled: !whole,
    count: whole ? 0 : count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW_PX,
    overscan: OVERSCAN,
  })
  const items = virtualizer.getVirtualItems()
  const total = virtualizer.getTotalSize()

  // Pin to the newest line after every change while following, including a re-measure that
  // changed the content height.
  useLayoutEffect(() => {
    if (!follow) return
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [follow, lines, rows, total])

  // Scroll to the focused line and flash it.
  const focusSeq = focus?.seq
  useLayoutEffect(() => {
    if (!focus) return
    const position = rows ? rows.indexOf(focus.line) : focus.line
    if (position < 0 || position >= count) return
    const reveal = () => {
      const el = scrollRef.current?.querySelector<HTMLElement>(`[data-line="${String(focus.line)}"]`)
      if (!el) return false
      // jsdom has no scrollIntoView.
      if (typeof el.scrollIntoView === 'function') el.scrollIntoView({ block: 'center' })
      el.focus({ preventScroll: true })
      return true
    }
    if (whole) reveal()
    else {
      // Place the line directly: right after a switch of view (results -> context) the
      // virtualiser's scrollToIndex alone can leave the view where it was (still pinned to the
      // bottom from following); its measurements, or the estimate, give the offset.
      const el = scrollRef.current
      const m = virtualizer.measurementsCache[position]
      if (el) {
        const start = m?.start ?? position * ESTIMATED_ROW_PX
        const size = m?.size ?? ESTIMATED_ROW_PX
        el.scrollTop = Math.max(0, start - (el.clientHeight - size) / 2)
      }
      virtualizer.scrollToIndex(position, { align: 'center' })
      // The row exists once the virtualiser has rendered the new window.
      if (!reveal()) requestAnimationFrame(() => reveal())
    }
    setFlashAbs(base + focus.line)
    const t = setTimeout(() => setFlashAbs(null), FLASH_MS)
    return () => clearTimeout(t)
    // Only a new request moves the view; later line changes must not re-scroll.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusSeq])

  const onScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    onFollowChange?.(el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_THRESHOLD_PX)
    const atTop = el.scrollTop <= 0
    if (atTop && !atTopRef.current) onLoadOlder?.()
    atTopRef.current = atTop
  }, [onFollowChange, onLoadOlder])

  const onToggle = useCallback(
    (line: number) => {
      setExpanded((prev) => {
        const next = new Set(prev)
        const abs = base + line
        if (next.has(abs)) next.delete(abs)
        else next.add(abs)
        return next
      })
    },
    [base],
  )

  const marker = dropped === false || dropped === 0 ? undefined : droppedText(dropped)
  const first = items[0]
  const last = items[items.length - 1]

  const row = (position: number, key: number | string | bigint, measure?: (el: Element | null) => void) => {
    const line = lineAt(position)
    return (
      <Row
        key={key}
        raw={lines[line] ?? ''}
        cache={cache}
        position={position}
        line={line}
        last={position === count - 1}
        highlights={highlights}
        activatable={onActivateLine !== undefined}
        onActivate={onActivateLine}
        expanded={expanded.has(base + line)}
        onToggle={onToggle}
        flash={flashAbs === base + line}
        measure={measure}
      />
    )
  }

  return (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      className={cn(
        'h-full min-w-0 overflow-y-auto bg-background/40 p-2 font-mono text-xs leading-relaxed',
        className,
      )}
      {...rest}
    >
      {placeholder ?? (
        <>
          {marker !== undefined && (
            <p data-pane-state="logs-truncated" className="mb-1 text-xs text-muted-foreground">
              {marker}
            </p>
          )}
          <div
            className={cn(
              'min-w-0 text-foreground/90',
              wrap ? 'break-words whitespace-pre-wrap' : 'whitespace-pre',
            )}
            style={
              whole
                ? undefined
                : { paddingTop: first?.start ?? 0, paddingBottom: last ? total - last.end : 0 }
            }
          >
            {whole
              ? Array.from({ length: count }, (_, i) => row(i, i))
              : items.map((item) => row(item.index, item.key, virtualizer.measureElement))}
          </div>
        </>
      )}
    </div>
  )
}

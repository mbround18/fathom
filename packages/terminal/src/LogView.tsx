/**
 * A scrolling, virtualised view over a log's lines.
 *
 * It does the three things every log pane needs, whatever the transport: it pins the newest line
 * while `follow` is on, it reports when the operator scrolls away (or back), and it says when
 * older lines were dropped. Status, empty and error states belong to the caller, which passes
 * one as `placeholder`.
 *
 * Only the visible window is in the DOM once a log is long (`FULL_RENDER_MAX`); a short log is
 * rendered whole, without the virtualiser, so its every line stays findable with the browser's own
 * search and is there before the first layout.
 */
import { useVirtualizer } from '@tanstack/react-virtual'
import { useCallback, useLayoutEffect, useRef, type HTMLAttributes, type ReactNode } from 'react'

import { cn } from '@companion/theme'

/** How close to the bottom (px) still counts as "at the bottom" for follow. */
export const FOLLOW_THRESHOLD_PX = 24
/** Up to this many lines are all rendered; past it, only the visible window and an overscan. */
export const FULL_RENDER_MAX = 1_000
const OVERSCAN = 20
const ESTIMATED_ROW_PX = 20

export type DroppedCount = number | boolean

export interface LogViewProps extends Omit<HTMLAttributes<HTMLDivElement>, 'onScroll'> {
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
}

export function defaultDroppedText(dropped: number | true): string {
  if (dropped === true) return 'Older lines dropped'
  return `${String(dropped)} older line${dropped === 1 ? '' : 's'} dropped`
}

/**
 * A row's text with the line break that separates it from the next, so the view's text (and a
 * copied selection) reads as the log did. A break at the end of a block draws no extra line.
 */
function withBreak(line: string | undefined, index: number, count: number): string {
  return index < count - 1 ? `${line ?? ''}\n` : (line ?? '')
}

export function LogView({
  lines,
  follow,
  wrap,
  onLoadOlder,
  onFollowChange,
  dropped = 0,
  droppedText = defaultDroppedText,
  placeholder,
  className,
  ...rest
}: LogViewProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const atTopRef = useRef(false)

  const whole = lines.length <= FULL_RENDER_MAX
  const virtualizer = useVirtualizer({
    enabled: !whole,
    count: whole ? 0 : lines.length,
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
  }, [follow, lines, total])

  const onScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    onFollowChange?.(el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_THRESHOLD_PX)
    const atTop = el.scrollTop <= 0
    if (atTop && !atTopRef.current) onLoadOlder?.()
    atTopRef.current = atTop
  }, [onFollowChange, onLoadOlder])

  const marker = dropped === false || dropped === 0 ? undefined : droppedText(dropped)
  const first = items[0]
  const last = items[items.length - 1]

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
              ? lines.map((line, i) => (
                  <div key={i} data-index={i} className="min-h-[1lh]">
                    {withBreak(line, i, lines.length)}
                  </div>
                ))
              : items.map((item) => (
                  <div
                    key={item.key}
                    data-index={item.index}
                    ref={virtualizer.measureElement}
                    className="min-h-[1lh]"
                  >
                    {withBreak(lines[item.index], item.index, lines.length)}
                  </div>
                ))}
          </div>
        </>
      )}
    </div>
  )
}

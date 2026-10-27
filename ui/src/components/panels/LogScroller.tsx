/**
 * The presentational half of a live log view: the scroll surface, auto-follow, the truncation
 * line, and the bar that says a stream ended.
 *
 * Extracted from `PodLogsTab` when 048 needed the same view over a container's output. What makes
 * a log pane trustworthy is not the transport — it is that it follows the newest line unless the
 * operator scrolled away, that it says "no output yet" rather than showing nothing, that it admits
 * when earlier lines were dropped, and that it tells you when the stream stopped instead of just
 * going quiet. Those four are the same whether the lines came from the API server or from a
 * container on this machine, so they live here once, and each caller brings its own transport.
 *
 * The scroll surface itself is `@companion/terminal`'s `LogExplorer`: ANSI colour, a level gutter
 * and facets, the log query language, and search results that jump to the line in context. This
 * adds Fathom's stream states, its wording, and the jump-to-latest button.
 */
import { ArrowClockwiseIcon, ArrowLineDownIcon, WarningIcon } from '@phosphor-icons/react'
import { useCallback, useState } from 'react'

import { ErrorState, FOLLOW_THRESHOLD_PX, Loading, LogExplorer } from '@companion/terminal'
import { Button } from '@companion/ui'
import { cn } from '@companion/theme'

export { FOLLOW_THRESHOLD_PX }

export type LogStreamStatus = 'starting' | 'streaming' | 'closed' | 'error'

export interface LogScrollerProps {
  lines: string[]
  /** How many earlier lines the buffer dropped; stated, never hidden. */
  truncated: number
  /**
   * The producer dropped earlier lines without saying how many.
   *
   * A container's follow bounds itself at the source, so what arrives is "there was a gap" and not
   * "there were 412 lines". An uncounted gap is still a gap the operator must be told about
   * (FR-034), so it gets its own sentence rather than being rounded to zero.
   */
  dropped?: boolean
  /** Bumped by the store on every append. */
  rev: number
  status: LogStreamStatus
  /** The backend's own words for a failure. */
  error?: string | undefined
  /** Why the stream closed, as the backend named it. */
  reason?: string | undefined
  /** What is being waited for while `status` is `starting`. */
  startingLabel: string
  /** Title for the error state that replaces the pane when nothing ever arrived. */
  errorTitle: string
  onRestart: () => void
  /** Kept as the pod value by default so existing views and their specs are untouched. */
  testId?: string
}

/** Fathom's wording for the dropped-lines marker. */
function droppedText(n: number | true): string {
  return n === true
    ? '… earlier lines were truncated; the runtime did not say how many'
    : `… ${String(n)} earlier line${n === 1 ? '' : 's'} truncated`
}

export function LogScroller({
  lines,
  truncated,
  dropped = false,
  status,
  error,
  reason,
  startingLabel,
  errorTitle,
  onRestart,
  testId = 'pod-logs-scroll',
}: LogScrollerProps) {
  const [follow, setFollow] = useState(true)
  const jumpToLatest = useCallback(() => setFollow(true), [])

  const failedEmpty = status === 'error' && lines.length === 0
  const placeholder =
    status === 'starting' ? (
      <Loading rows={6} label={startingLabel} />
    ) : lines.length === 0 ? (
      <p
        role="status"
        data-pane-state="logs-empty"
        className="font-sans text-xs text-muted-foreground"
      >
        {status === 'streaming' ? 'No log output yet.' : 'No log output.'}
      </p>
    ) : undefined

  return (
    <>
      <div className="relative flex min-h-0 flex-1 flex-col">
        {failedEmpty ? (
          <ErrorState title={errorTitle} detail={error ?? reason} onRetry={onRestart} />
        ) : (
          <LogExplorer
            lines={lines}
            follow={follow}
            wrap
            onFollowChange={setFollow}
            dropped={truncated > 0 ? truncated : dropped}
            droppedText={droppedText}
            placeholder={placeholder}
            data-testid={testId}
            data-stream-status={status}
            className="min-h-0 flex-1"
          />
        )}
        {!follow && !failedEmpty && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={jumpToLatest}
            className={cn(
              'absolute right-3 bottom-3 border-accent-neon/60 bg-background/90 shadow-[0_0_16px_-4px_var(--accent-neon)]',
            )}
          >
            <ArrowLineDownIcon data-icon="inline-start" /> Jump to latest
          </Button>
        )}
      </div>
      {(status === 'closed' || (status === 'error' && !failedEmpty)) && (
        <StreamEndBar status={status} reason={reason} error={error} onRestart={onRestart} />
      )}
    </>
  )
}

/** Says a stream is over, and which kind of over it was. */
export function StreamEndBar({
  status,
  reason,
  error,
  onRestart,
}: {
  status: LogStreamStatus
  reason?: string | undefined
  error?: string | undefined
  onRestart: () => void
}) {
  const failed = status === 'error'
  const text = failed
    ? `Stream failed${error ? `: ${error}` : ''}`
    : reason === 'stopped'
      ? 'Stream stopped'
      : `Stream ended${reason && reason !== 'ended' ? `: ${reason}` : ''}`
  return (
    <div
      role={failed ? 'alert' : 'status'}
      data-pane-state={failed ? 'stream-error' : 'stream-ended'}
      className={cn(
        'flex shrink-0 items-center gap-2 border-t px-3 py-1.5 text-xs',
        failed
          ? 'border-status-err/40 bg-status-err/10 text-status-err'
          : 'border-status-warn/40 bg-status-warn/10 text-status-warn',
      )}
    >
      <WarningIcon aria-hidden="true" className="size-4 shrink-0" />
      <span className="min-w-0 flex-1 break-words">{text}</span>
      <Button type="button" variant="ghost" size="xs" onClick={onRestart}>
        <ArrowClockwiseIcon data-icon="inline-start" /> {failed ? 'Retry' : 'Restart'}
      </Button>
    </div>
  )
}

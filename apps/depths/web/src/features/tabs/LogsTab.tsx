/**
 * One log stream (FR-026): follow, pause, search, level facets, a time window and wrap, over the
 * tab's ring of lines. Lines are kept raw; the shared log view renders SGR colour as styled text,
 * removes every other control sequence and never renders a line as markup. Activating a search
 * result shows it in context: the whole log, scrolled to that line.
 */
import { ArrowLineDownIcon, ArrowsClockwiseIcon, PauseIcon, PlayIcon, TextAlignLeftIcon } from '@phosphor-icons/react'
import { LogExplorer } from '@companion/terminal'
import { Button, Input } from '@companion/ui'
import { useCallback, useMemo, useState } from 'react'

import type { TabSpec } from './layout'
import { localInputMs } from './logtext'
import { retryLogs, useStreams, type LogLine } from './streams'

const PHASE_TEXT = { opening: 'Opening…', live: 'Live', ended: 'Ended', failed: 'Failed', denied: 'Not permitted' } as const

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString()
}

export function LogsTab({ tab }: { tab: Extract<TabSpec, { kind: 'logs' }> }) {
  const stream = useStreams((s) => s.logs[tab.id])
  const [follow, setFollow] = useState(true)
  const [wrap, setWrap] = useState(true)
  const [paused, setPaused] = useState<{ lines: LogLine[]; total: number } | null>(null)
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')

  const rev = stream?.rev ?? 0
  const ring = stream?.lines
  const total = ring ? ring.truncated + ring.length : 0
  // `rev` marks a change to the (mutable) ring.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const live = useMemo(() => ring?.toArray() ?? [], [ring, rev])
  const source = paused?.lines ?? live
  const texts = useMemo(() => source.map((l) => l.text), [source])

  const fromMs = localInputMs(from)
  const toMs = localInputMs(to)
  const windowed = fromMs !== null || toMs !== null
  const inWindow = useCallback(
    (i: number) => {
      const at = source[i]?.at ?? null
      if (fromMs !== null && (at === null || at < fromMs)) return false
      if (toMs !== null && (at === null || at > toMs)) return false
      return true
    },
    [source, fromMs, toMs],
  )

  const phase = stream?.phase ?? 'opening'
  const newSincePause = paused ? total - paused.total : 0

  const placeholder =
    phase === 'denied' ? (
      <p role="status" data-pane-state="not-permitted" className="text-xs text-muted-foreground">
        {stream?.reason ?? 'Not permitted.'}
      </p>
    ) : source.length === 0 ? (
      <p role="status" data-pane-state={phase === 'opening' ? 'loading' : 'empty'} className="text-xs text-muted-foreground">
        {phase === 'opening' ? 'Opening logs…' : 'No log lines yet.'}
      </p>
    ) : undefined

  const toolbarStart = (
    <>
      <span data-field="phase" data-phase={phase} className="px-1 text-muted-foreground">
        {PHASE_TEXT[phase]}
      </span>
      {stream?.notice && (
        <span role="status" data-notice className="px-1 text-status-ok">
          {stream.notice}
        </span>
      )}
      <Button
        variant={follow ? 'outline' : 'ghost'}
        size="icon-xs"
        aria-label="Follow"
        aria-pressed={follow}
        title="Follow the newest line"
        onClick={() => setFollow((f) => !f)}
      >
        <ArrowLineDownIcon aria-hidden />
      </Button>
      <Button
        variant={paused ? 'outline' : 'ghost'}
        size="icon-xs"
        aria-label={paused ? 'Resume' : 'Pause'}
        aria-pressed={paused !== null}
        title={paused ? 'Resume' : 'Pause'}
        onClick={() => setPaused((p) => (p ? null : { lines: live, total }))}
      >
        {paused ? <PlayIcon aria-hidden /> : <PauseIcon aria-hidden />}
      </Button>
      <Button
        variant={wrap ? 'outline' : 'ghost'}
        size="icon-xs"
        aria-label="Wrap lines"
        aria-pressed={wrap}
        title="Wrap long lines"
        onClick={() => setWrap((w) => !w)}
      >
        <TextAlignLeftIcon aria-hidden />
      </Button>
    </>
  )

  const toolbarEnd = (
    <>
      <span className="flex items-center gap-1 text-muted-foreground">
        <span aria-hidden>From</span>
        <Input type="datetime-local" step="1" aria-label="From time" value={from} onChange={(e) => setFrom(e.target.value)} className="h-6 w-auto text-xs" />
      </span>
      <span className="flex items-center gap-1 text-muted-foreground">
        <span aria-hidden>To</span>
        <Input type="datetime-local" step="1" aria-label="To time" value={to} onChange={(e) => setTo(e.target.value)} className="h-6 w-auto text-xs" />
      </span>
      {paused && newSincePause > 0 && (
        <span data-field="new-lines" className="px-1 text-status-warn">
          {newSincePause} new
        </span>
      )}
    </>
  )

  const banner = (phase === 'ended' || phase === 'failed') && (
    <div role="status" data-ended className="flex min-w-0 items-center gap-2 border-b border-border px-2 py-1 text-xs text-muted-foreground">
      <span className="min-w-0 flex-1 break-words">
        {stream?.endedAt != null && `Ended ${clock(stream.endedAt)}: `}
        {stream?.reason}
      </span>
      <Button variant="outline" size="xs" onClick={() => retryLogs(tab.id)}>
        <ArrowsClockwiseIcon aria-hidden /> Reopen
      </Button>
    </div>
  )

  return (
    <div data-tab-body="logs" className="flex h-full min-h-0 flex-col">
      <LogExplorer
        lines={texts}
        follow={follow && !paused}
        wrap={wrap}
        onFollowChange={setFollow}
        dropped={ring?.truncated ?? 0}
        placeholder={placeholder}
        toolbarStart={toolbarStart}
        toolbarEnd={toolbarEnd}
        banner={banner}
        filter={inWindow}
        filterActive={windowed}
        data-testid="log-view"
        className="min-h-0 flex-1"
      />
    </div>
  )
}

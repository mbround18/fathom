/**
 * An interactive shell in a container (FR-027), on xterm.js. Loaded lazily: this module pulls in
 * xterm and its stylesheet, which no other page needs.
 *
 * The session belongs to the streams store, not to this component, so hiding the tab (or moving it
 * to another pane) keeps the shell and replays its output when shown again. When the session ends
 * the tab says when and why, and offers a new one.
 */
import '@companion/terminal/terminal.css'

import { ArrowsClockwiseIcon } from '@phosphor-icons/react'
import { encodeInput, TerminalSurface, type TerminalTransport } from '@companion/terminal'
import { Button } from '@companion/ui'
import { useCallback, useMemo } from 'react'

import type { TabSpec } from './layout'
import { newShellSession, shellInput, shellResize, useStreams } from './streams'

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString()
}

function subscribe(onChange: () => void): () => void {
  return useStreams.subscribe(onChange)
}

export default function ShellTab({ tab }: { tab: Extract<TabSpec, { kind: 'shell' }> }) {
  const stream = useStreams((s) => s.shells[tab.id])
  const gen = stream?.gen ?? 0
  const id = tab.id

  const transport = useMemo<TerminalTransport>(
    () => ({
      key: `${id}:${String(gen)}`,
      // The store owns the session: it was opened with the tab, and closes with it.
      start: () => undefined,
      stop: () => undefined,
      input: (data) => shellInput(id, encodeInput(data)),
      current: () => useStreams.getState().shells[id],
    }),
    [id, gen],
  )
  const onResize = useCallback((cols: number, rows: number) => shellResize(id, cols, rows), [id])

  const ended = stream && (stream.status === 'ended' || stream.status === 'error')

  return (
    <div data-tab-body="shell" data-shell-status={stream?.status ?? 'attaching'} className="flex h-full min-h-0 flex-col">
      {ended && (
        <div role="status" data-ended className="flex min-w-0 items-center gap-2 border-b border-border px-2 py-1 text-xs text-muted-foreground">
          <span data-field="end-reason" className="min-w-0 flex-1 break-words">
            {stream.endedAt !== null ? `Session ended ${clock(stream.endedAt)}: ` : 'Session ended: '}
            {stream.reason}
          </span>
          {stream.offerNew && (
            <Button variant="outline" size="xs" onClick={() => newShellSession(id)}>
              <ArrowsClockwiseIcon aria-hidden /> New session
            </Button>
          )}
        </div>
      )}
      <TerminalSurface transport={transport} subscribe={subscribe} onResize={onResize} testId={`terminal-${id}`} />
    </div>
  )
}

/**
 * A stack's latest operation (US2 T056): output streaming live on `@companion/terminal`'s
 * `LogView`, a final outcome line, and a Cancel button while it runs. A failure shows docker's
 * stderr verbatim (FR-013). The stack's status is not touched here: it moves when the server's
 * `StacksChanged` says so.
 */
import { LogView } from '@companion/terminal'
import { Button } from '@companion/ui'
import { useMemo, useState } from 'react'

import { KIND_LABEL, outcomeLabel } from './argv'
import { useOps } from './ops-store'

/** Most recent lines kept on screen; older ones are counted as dropped. */
const MAX_LINES = 10_000

function toLines(text: string): { lines: string[]; dropped: number } {
  const all = text.replace(/\r\n/g, '\n').replace(/\n$/, '').split('\n')
  if (text === '') return { lines: [], dropped: 0 }
  const dropped = Math.max(0, all.length - MAX_LINES)
  return { lines: dropped > 0 ? all.slice(dropped) : all, dropped }
}

export function OperationOutput({ stack }: { stack: string }) {
  const run = useOps((s) => s.ops[stack])
  const cancelOp = useOps((s) => s.cancelOp)
  const clearOp = useOps((s) => s.clearOp)
  const [follow, setFollow] = useState(true)
  const { lines, dropped } = useMemo(() => toLines(run?.text ?? ''), [run?.text])

  if (!run) return null
  const outcome = run.outcome
  const running = outcome === null
  const tone =
    outcome === null
      ? 'text-muted-foreground'
      : outcome.kind === 'succeeded'
        ? 'text-status-ok'
        : outcome.kind === 'failed'
          ? 'text-status-err'
          : 'text-status-warn'

  return (
    <section
      data-operation={run.kind}
      data-op-state={outcome === null ? 'running' : outcome.kind}
      aria-label="Operation output"
      className="flex min-w-0 flex-col border border-border bg-card"
    >
      <header className="flex min-w-0 items-center gap-2 border-b border-border px-2 py-1">
        <h2 className="min-w-0 flex-1 truncate font-display text-xs font-semibold tracking-wider uppercase">
          {KIND_LABEL[run.kind]}
        </h2>
        <span role="status" data-field="op-status" className={`shrink-0 text-xs ${tone}`}>
          {outcome === null ? (run.cancelling ? 'Cancelling…' : 'Running…') : outcomeLabel(outcome)}
        </span>
        {running && (
          <Button
            variant="outline"
            size="xs"
            disabled={run.cancelling || run.operationId === null}
            onClick={() => cancelOp(stack)}
          >
            Cancel
          </Button>
        )}
        {(!running || run.interrupted) && (
          <Button variant="ghost" size="xs" onClick={() => clearOp(stack)}>
            Dismiss
          </Button>
        )}
      </header>
      <LogView
        data-op-output
        role="log"
        aria-live="polite"
        lines={lines}
        follow={follow}
        wrap
        dropped={dropped}
        onFollowChange={setFollow}
        className="max-h-64 min-h-12"
        placeholder={
          lines.length === 0 ? <p className="text-xs text-muted-foreground">{running ? 'Waiting for output…' : 'No output.'}</p> : undefined
        }
      />
      {outcome?.kind === 'failed' && outcome.stderr !== '' && (
        <div className="border-t border-border px-2 py-1">
          <p className="text-xs text-status-err">Error output</p>
          <pre data-field="stderr" className="max-h-48 overflow-auto font-mono text-xs break-all whitespace-pre-wrap text-status-err">
            {outcome.stderr}
          </pre>
        </div>
      )}
      {run.interrupted && running && (
        <p role="note" className="border-t border-border px-2 py-1 text-xs text-status-warn">
          The connection dropped while this ran; later output was not received. The stack status above is current.
        </p>
      )}
    </section>
  )
}

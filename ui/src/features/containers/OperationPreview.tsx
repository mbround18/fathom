/**
 * 048-docker-management US2: the preview, the confirmation, and the outcome.
 *
 * This component is the whole of what FR-018 through FR-023 promise, so it is worth saying what it
 * deliberately does not do:
 *
 * - It does not build a command. The argv it renders came from the backend, which stored it and
 *   will run that same stored vector; this side never derives one. A second derivation here would
 *   drift from the first, and the preview would become a story about the operation rather than the
 *   operation itself.
 * - It does not count. Every object that disappears is named on its own line (FR-023). "4 volumes
 *   will be removed" is exactly the sentence that has cost people work on a shared daemon.
 * - It does not change state optimistically (FR-021). The row the operator acted on keeps saying
 *   what it said until the runtime's own outcome arrives. A list that shows "stopped" before the
 *   daemon agrees is a list that has started guessing.
 * - It does not accept a near-miss confirmation. The typed string must equal the one the preview
 *   asked for, and the comparison the backend makes is the one that counts — this is only so the
 *   operator is not sent to a refusal they could see coming.
 */
import { WarningOctagonIcon } from '@phosphor-icons/react'
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import type { ObjectRef, OperationOutcome, OperationPreview as Preview } from '@/protocol/types'
import { useStore } from '@/store'

import { refusalLabel, sizeLabel } from './labels'

function refName(object: ObjectRef): string {
  return object.name === '' ? object.id : object.name
}

function Removals({ preview }: { preview: Preview }) {
  const total = sizeLabel(preview.total_size_bytes)
  return (
    <section className="min-w-0">
      <h4 className="font-display text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
        Will be removed
        {total !== null && <span data-testid="operation-total-size"> — {total} reclaimed</span>}
      </h4>
      <ul data-testid="operation-removals" className="mt-1 space-y-0.5">
        {preview.removals.map((line) => {
          const size = sizeLabel(line.size_bytes)
          return (
            <li
              key={`${line.object.class}:${line.object.id}`}
              data-testid={`operation-removal-${line.object.id}`}
              className="flex min-w-0 items-baseline gap-2 font-mono text-xs"
            >
              <span className="shrink-0 text-[10px] tracking-wider text-muted-foreground uppercase">
                {line.object.class}
              </span>
              <span className="min-w-0 truncate">{refName(line.object)}</span>
              {line.project !== null && (
                <span className="shrink-0 text-muted-foreground">{line.project}</span>
              )}
              {size !== null && <span className="ml-auto shrink-0 text-muted-foreground">{size}</span>}
            </li>
          )
        })}
      </ul>
    </section>
  )
}

function Outcome({ outcome }: { outcome: OperationOutcome }) {
  return (
    <div data-testid="operation-outcome" data-overall={outcome.overall} className="min-w-0">
      <ul className="space-y-0.5">
        {outcome.objects.map((o) => (
          <li
            key={`${o.object.class}:${o.object.id}`}
            data-testid={`operation-outcome-${o.object.id}`}
            data-status={o.status}
            className="flex min-w-0 flex-col gap-0.5 font-mono text-xs"
          >
            <span className="flex min-w-0 items-baseline gap-2">
              <span
                className={cn(
                  'shrink-0 text-[10px] tracking-wider uppercase',
                  o.status === 'succeeded' && 'text-status-ok',
                  o.status === 'failed' && 'text-status-err',
                  o.status === 'skipped' && 'text-muted-foreground',
                )}
              >
                {o.status}
              </span>
              <span className="min-w-0 truncate">{refName(o.object)}</span>
            </span>
            {/* The runtime's own words. A per-object failure rewritten in our voice loses the one
                thing it is useful for: naming what the daemon actually objected to. */}
            {o.error !== null && (
              <span className="pl-2 break-words text-status-err">{o.error}</span>
            )}
          </li>
        ))}
      </ul>
      {/* FR-024: a difference between what was previewed and what the re-read inventory shows is
          reported, never swallowed. */}
      {outcome.discrepancies.length > 0 && (
        <ul data-testid="operation-discrepancies" className="mt-2 space-y-0.5 text-xs">
          {outcome.discrepancies.map((d) => (
            <li key={`${d.object.class}:${d.object.id}`} className="text-status-warn">
              {refName(d.object)}: expected {d.expected}, found {d.observed}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

export function OperationPreview() {
  const operation = useStore((s) => s.containerRuntime.operation)
  if (!operation) return null
  // Keyed on the preview: a new preview gets a new component and so an empty field, rather than
  // one preview's typed confirmation standing in for another's.
  return <Body key={operation.preview?.preview_id ?? operation.requestId} />
}

function Body() {
  const operation = useStore((s) => s.containerRuntime.operation)
  const issue = useStore((s) => s.issueContainerOperation)
  const dismiss = useStore((s) => s.dismissContainerOperation)
  const [typed, setTyped] = useState('')

  if (!operation) return null

  const preview = operation.preview
  const required = preview?.required_confirmation ?? null
  const satisfied = required === null || typed === required

  return (
    <div
      data-testid="operation-preview"
      data-phase={operation.phase}
      className="flex min-w-0 flex-col gap-3 border border-border bg-card p-3"
    >
      {operation.phase === 'previewing' && (
        <p className="text-xs text-muted-foreground">Working out what this would run…</p>
      )}

      {preview && (
        <>
          {/* The literal argv, program first, in a monospace block. Not a prose rendering of it:
              the operator is being asked to approve a command, so they are shown the command. */}
          <div className="min-w-0">
            <h4 className="font-display text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
              Will run
            </h4>
            <pre
              data-testid="operation-command"
              className="mt-1 overflow-x-auto bg-muted/40 p-2 font-mono text-xs"
            >
              {preview.command.join(' ')}
            </pre>
          </div>

          {preview.removals.length > 0 && <Removals preview={preview} />}

          {preview.blast_radius.length > 0 && (
            <p data-testid="operation-blast-radius" className="text-xs">
              <span className="text-muted-foreground">Reaches: </span>
              {preview.blast_radius.join(', ')}
            </p>
          )}

          {/* FR-025: objects inside the blast radius that will be left alone, and why. Excluded,
              not merely flagged — the command above does not name them. */}
          {preview.excluded.length > 0 && (
            <ul data-testid="operation-excluded" className="space-y-0.5 text-xs">
              {preview.excluded.map((e) => (
                <li key={`${e.object.class}:${e.object.id}`} className="text-muted-foreground">
                  {refName(e.object)} left alone — {e.reason.detail} has not been adopted
                </li>
              ))}
            </ul>
          )}

          {/* FR-031: a container that backs a cluster the operator is connected to is said out
              loud, before the fact rather than after. */}
          {preview.breaks_clusters.length > 0 && (
            <div data-testid="operation-breaks-clusters" className="flex gap-2 text-xs text-status-warn">
              <WarningOctagonIcon aria-hidden="true" className="mt-0.5 shrink-0" />
              <ul className="min-w-0 space-y-0.5">
                {preview.breaks_clusters.map((b) => (
                  <li key={b.cluster}>
                    {b.cluster} loses a node this acts on
                    {b.connected && ' — and you are connected to it right now'}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <p className="text-xs text-muted-foreground">
            Derived from the inventory read at {preview.as_of}.
          </p>

          {required !== null && operation.phase === 'previewed' && (
            <label htmlFor="operation-confirmation" className="flex min-w-0 flex-col gap-1 text-xs">
              <span>
                Type <span className="font-mono">{required}</span> to confirm
              </span>
              <Input
                id="operation-confirmation"
                data-testid="operation-confirmation"
                value={typed}
                autoComplete="off"
                onChange={(e) => setTyped(e.target.value)}
              />
            </label>
          )}
        </>
      )}

      {operation.phase === 'refused' && operation.refusal && (
        <p data-testid="operation-refusal" data-kind={operation.refusal.kind} className="text-xs text-status-err">
          {refusalLabel(operation.refusal)}
        </p>
      )}

      {operation.outcome && <Outcome outcome={operation.outcome} />}

      <div className="flex shrink-0 gap-2">
        {operation.phase === 'previewed' && (
          <Button
            size="sm"
            variant={preview?.destructive === true ? 'destructive' : 'default'}
            data-testid="operation-issue"
            disabled={!satisfied}
            onClick={() => issue(required === null ? null : typed)}
          >
            {preview?.destructive === true ? 'Remove' : 'Run it'}
          </Button>
        )}
        {operation.phase === 'issued' && (
          <span data-testid="operation-running" className="text-xs text-muted-foreground">
            Running…
          </span>
        )}
        <Button size="sm" variant="ghost" data-testid="operation-dismiss" onClick={dismiss}>
          {operation.phase === 'finished' || operation.phase === 'refused' ? 'Close' : 'Cancel'}
        </Button>
      </div>
    </div>
  )
}

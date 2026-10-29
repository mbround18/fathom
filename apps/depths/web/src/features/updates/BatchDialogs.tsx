/**
 * US2 (T050): the batch preview, then the run as it happens.
 *
 * - The preview lists each stack in the order it runs: the services to recreate with their old and
 *   new image, the services left alone and why, the steps, and the 051 findings to acknowledge.
 * - With nothing to recreate it says everything is up to date, and why per stack: information,
 *   not an error.
 * - The run shows each stack's outcome and live output, and a summary naming every stack that did
 *   not update. Stop lets the stack in progress finish (or roll back) and skips the rest.
 */
import { FingerprintIcon, StopIcon } from '@phosphor-icons/react'
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useRef } from 'react'

import type { PasskeyRequest, StackUpdatePlan } from '@/protocol/generated'

import { availableText, outcomeText, outcomeTone, runningText, triggerText } from './model'
import { useUpdates, type LiveRun } from './store'

/** 054: the preview asks for a passkey on Confirm; `note` says why the last try did not go through. */
export function StepUpNote({ stepUp, verifying, note }: { stepUp: PasskeyRequest | null; verifying: boolean; note: string | null }) {
  return (
    <>
      {stepUp && (
        <p data-step-up className="flex items-start gap-1 text-xs text-muted-foreground">
          <FingerprintIcon aria-hidden className="mt-0.5 size-3 shrink-0" />
          <span>
            {verifying
              ? 'Follow your browser’s passkey prompt…'
              : 'Confirming asks for your passkey: this needs a fresh check that it’s you.'}
          </span>
        </p>
      )}
      {note && (
        <p role="alert" data-refusal className="border border-status-warn/60 bg-status-warn/10 p-1.5 text-xs text-status-warn">
          {note}
        </p>
      )}
    </>
  )
}

export function BatchDialog() {
  const flow = useUpdates((s) => s.batch)
  const acknowledge = useUpdates((s) => s.acknowledge)
  const confirm = useUpdates((s) => s.confirmBatch)
  const cancel = useUpdates((s) => s.cancelBatch)
  const open = flow.kind !== 'idle'
  const shown = flow.kind === 'preview' || flow.kind === 'verifying' || flow.kind === 'confirming' ? flow : null
  const count =
    shown?.preview.stacks.length ?? (flow.kind === 'previewing' || flow.kind === 'up_to_date' ? flow.stacks.length : 0)
  const done = flow.kind === 'refused' || flow.kind === 'up_to_date'
  const blockedByAck = shown !== null && shown.preview.ackRequired && !shown.acknowledged
  return (
    <Dialog open={open} onOpenChange={(o) => !o && cancel()}>
      <DialogContent data-testid="updates-batch-dialog" className="max-h-[90dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            Update {String(count)} {count === 1 ? 'stack' : 'stacks'}
          </DialogTitle>
          <DialogDescription>
            Stacks update one at a time, in this order. A stack whose new image fails its health check is rolled back.
          </DialogDescription>
        </DialogHeader>
        {flow.kind === 'previewing' ? (
          <p role="status" className="text-xs text-muted-foreground">
            Preparing the update…
          </p>
        ) : flow.kind === 'refused' ? (
          <p role="alert" data-refusal className="text-xs text-status-err">
            {flow.reason}
          </p>
        ) : flow.kind === 'up_to_date' ? (
          <UpToDate stacks={flow.stacks} />
        ) : shown ? (
          <>
            <ol className="flex flex-col gap-2">
              {shown.preview.stacks.map((p, i) => (
                <PlanItem key={p.stack} plan={p} order={i + 1} />
              ))}
            </ol>
            {shown.preview.ackRequired && (
              <label className="flex items-start gap-1.5 border border-status-warn/60 p-1.5 text-xs">
                <input
                  type="checkbox"
                  data-field="acknowledge"
                  className="mt-0.5"
                  checked={shown.acknowledged}
                  disabled={shown.kind !== 'preview'}
                  onChange={(e) => acknowledge(e.target.checked)}
                />
                <span>I have read the findings above and want to update these stacks anyway.</span>
              </label>
            )}
            <StepUpNote stepUp={shown.stepUp} verifying={shown.kind === 'verifying'} note={shown.note} />
          </>
        ) : null}
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={cancel}>
            {done ? 'Close' : 'Cancel'}
          </Button>
          {shown && (
            <Button size="sm" data-action="confirm-batch" disabled={shown.kind !== 'preview' || blockedByAck} onClick={confirm}>
              {shown.kind === 'verifying' ? 'Waiting for passkey…' : shown.kind === 'confirming' ? 'Starting…' : 'Update'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function UpToDate({ stacks }: { stacks: readonly StackUpdatePlan[] }) {
  return (
    <div className="flex flex-col gap-1 text-xs">
      <p role="status" data-up-to-date>
        Everything is up to date.
      </p>
      <ul aria-label="Left as they are" className="flex flex-col gap-0.5 text-muted-foreground">
        {stacks.flatMap((p) =>
          p.skipped.map(([service, why]) => (
            <li key={`${p.stack}/${service}`} data-plan-skipped={`${p.stack}/${service}`} className="min-w-0 break-words">
              {p.stack}/{service}: {why}
            </li>
          )),
        )}
      </ul>
    </div>
  )
}

function PlanItem({ plan, order }: { plan: StackUpdatePlan; order: number }) {
  return (
    <li data-plan={plan.stack} className="flex min-w-0 flex-col gap-1 border border-border p-1.5 text-xs">
      <p className="font-semibold">
        <span className="text-muted-foreground tabular-nums">{String(order)}. </span>
        {plan.stack}
      </p>
      {plan.services.length === 0 ? (
        <p className="text-muted-foreground">Nothing to recreate: every service is up to date or left as it is.</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {plan.services.map((s) => (
            <li key={s.service} data-plan-service={s.service} className="min-w-0 break-words">
              <span className="font-semibold">{s.service}</span>{' '}
              <span className="font-mono text-muted-foreground">{runningText(s)}</span> →{' '}
              <span className="font-mono">{availableText(s) ?? '—'}</span>
            </li>
          ))}
        </ul>
      )}
      <p data-field="recreate-note" className="text-muted-foreground">
        Only services with a newer image are recreated.
      </p>
      {plan.skipped.length > 0 && (
        <ul aria-label="Left as they are" className="flex flex-col gap-0.5 text-muted-foreground">
          {plan.skipped.map(([service, why]) => (
            <li key={service} data-plan-skipped={service} className="min-w-0 break-words">
              {service}: {why}
            </li>
          ))}
        </ul>
      )}
      {plan.findings.length > 0 && (
        <ul data-field="findings" aria-label="Compose findings" className="flex flex-col gap-0.5">
          {plan.findings.map((f) => (
            <li
              key={`${f.service}-${f.code}`}
              className={`min-w-0 break-words ${f.severity === 'escape' ? 'text-status-err' : 'text-status-warn'}`}
            >
              {f.service}: {f.message}
            </li>
          ))}
        </ul>
      )}
      {plan.argv.length > 0 && (
        <details>
          <summary className="cursor-pointer text-muted-foreground">Steps ({String(plan.argv.length)})</summary>
          <pre className="mt-0.5 overflow-x-auto border border-border p-1 font-mono">{plan.argv.map((a) => a.join(' ')).join('\n')}</pre>
        </details>
      )}
    </li>
  )
}

/** Stacks that did not update, as the summary names them. */
function notUpdated(run: LiveRun): string[] {
  return run.stacks.flatMap((s) => (s.outcome !== null && s.outcome.kind !== 'updated' ? [`${s.stack} (${outcomeText(s.outcome)})`] : []))
}

export function RunDialog() {
  const run = useUpdates((s) => s.run)
  const stop = useUpdates((s) => s.stopRun)
  const close = useUpdates((s) => s.closeRun)
  if (run === null) return null
  const done = run.stacks.filter((s) => s.outcome?.kind === 'updated').length
  const others = notUpdated(run)
  const current = run.stacks.find((s) => s.operationId !== null && s.outcome === null)
  return (
    <Dialog open onOpenChange={(o) => !o && close()}>
      <DialogContent data-testid="updates-run-dialog" data-run-finished={run.finished ? 'true' : 'false'} className="max-h-[90dvh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{run.finished ? 'Update finished' : 'Updating…'}</DialogTitle>
          <DialogDescription>
            Started {triggerText(run.trigger)}. Every stack is recorded in the audit log.
          </DialogDescription>
        </DialogHeader>
        <ol className="flex flex-col gap-1.5">
          {run.stacks.map((s) => (
            <li key={s.stack} data-run-stack={s.stack} className="flex min-w-0 flex-col gap-0.5 text-xs">
              <p className="flex min-w-0 flex-wrap items-center gap-1.5">
                <span className="font-semibold">{s.stack}</span>
                {s.outcome ? (
                  <span data-field="outcome" className={`inline-flex h-5 items-center border px-1.5 ${outcomeTone(s.outcome)}`}>
                    {outcomeText(s.outcome)}
                  </span>
                ) : (
                  <span data-field="outcome" className="text-muted-foreground">
                    {s.operationId ? 'running…' : 'waiting'}
                  </span>
                )}
              </p>
              {s.output !== '' && <RunOutput text={s.output} live={s === current} />}
            </li>
          ))}
        </ol>
        {run.finished && (
          <p role="status" data-field="run-summary" className="border border-border p-1.5 text-xs">
            {String(done)} of {String(run.stacks.length)} updated.
            {others.length > 0 && <> Not updated: {others.join(', ')}.</>}
          </p>
        )}
        <DialogFooter>
          {run.finished ? (
            <Button size="sm" data-action="close-run" onClick={close}>
              Close
            </Button>
          ) : (
            <Button variant="outline" size="sm" data-action="stop-run" disabled={run.cancelling || !current} onClick={stop}>
              <StopIcon aria-hidden />
              {run.cancelling ? 'Stopping…' : 'Stop: skip the rest'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function RunOutput({ text, live }: { text: string; live: boolean }) {
  const ref = useRef<HTMLPreElement>(null)
  useEffect(() => {
    if (live && ref.current) ref.current.scrollTop = ref.current.scrollHeight
  }, [text, live])
  return (
    <pre
      ref={ref}
      data-field="output"
      aria-live={live ? 'polite' : 'off'}
      className="max-h-32 overflow-auto border border-border p-1 font-mono text-[11px] whitespace-pre-wrap break-all"
    >
      {text.split('\n').map((line, i, all) =>
        line.startsWith('kept: in use') ? (
          <span key={i} data-kept className="text-status-warn">
            {line}
            {i < all.length - 1 ? '\n' : ''}
          </span>
        ) : (
          <span key={i}>
            {line}
            {i < all.length - 1 ? '\n' : ''}
          </span>
        ),
      )}
    </pre>
  )
}

/**
 * The approval panel on a held run (spec 064 US1, US2): what its policy needs, the approvals so far,
 * when its window closes, and Approve and Deny for someone who may answer (Deny only for whoever
 * started a four-eyes run). A denied run shows who denied it and why. The variant comes from
 * `approvalView`; the server decides again when the person confirms, and its refusal shows in the
 * dialog in its own words.
 *
 * A run whose approval is over (queued, denied, expired) keeps the panel as its history.
 */
import { CheckIcon, FingerprintIcon, ProhibitIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useId } from 'react'

import { useSession } from '@/app/session-store'
import { when } from '@/features/settings/updates'
import { TermTip } from '@/help/TermTip'
import type { ApprovalEntry, RunApproval, RunInfo } from '@/protocol/wire'
import { PreviewDetails } from '@/ui/ConfirmDialog'
import { Dialog } from '@/ui/Dialog'
import { FIELD, H2, LABEL } from '@/ui/form'
import { RefusalText } from '@/ui/RefusalText'

import { approvalView, relative } from './approval'
import { MAX_REASON, useDecision } from './decision'

function plural(n: number, one: string): string {
  return n === 1 ? one : `${one}s`
}

/** A time as "5 minutes ago", with the exact time on hover. */
function At({ at }: { at: string }) {
  return (
    <time dateTime={at} title={when(at)} className="tabular-nums">
      {relative(at)}
    </time>
  )
}

/** Who answered, and as what. */
function Who({ entry }: { entry: ApprovalEntry }) {
  return (
    <>
      <span className="font-medium">{entry.name}</span>
      {entry.role && <span className="text-muted-foreground"> ({entry.role.replace(/_/g, ' ')})</span>}
    </>
  )
}

function Approvals({ approval }: { approval: RunApproval }) {
  if (approval.approvals.length === 0) return null
  return (
    <ul aria-label="Approvals" className="flex min-w-0 flex-col border border-border text-xs">
      {approval.approvals.map((a) => (
        <li key={a.user} data-approval={a.user} className="flex min-w-0 items-start gap-1.5 border-t border-border px-2 py-1 first:border-t-0">
          <CheckIcon aria-hidden className="mt-0.5 size-3 shrink-0 text-status-ok" />
          <span className="min-w-0 break-words">
            <Who entry={a} />{' '}
            <span className="text-muted-foreground">
              <At at={a.at} />
            </span>
          </span>
        </li>
      ))}
    </ul>
  )
}

/** Who denied the run, when, and their reason as they wrote it. */
function Denial({ denial }: { denial: ApprovalEntry }) {
  return (
    <div data-approval-denial className="flex min-w-0 flex-col gap-1 border border-status-err/40 px-2 py-1.5 text-xs">
      <p className="flex min-w-0 flex-wrap items-baseline gap-x-1.5">
        <ProhibitIcon aria-hidden className="size-3 shrink-0 self-center text-status-err" />
        <span className="text-status-err">Denied by</span>{' '}
        <span className="min-w-0 break-words">
          <Who entry={denial} />
        </span>{' '}
        <span className="text-muted-foreground">
          <At at={denial.at} />
        </span>
      </p>
      {denial.reason && (
        <p data-approval-reason className="min-w-0 border-l-2 border-border pl-2 break-words whitespace-pre-wrap">
          {denial.reason}
        </p>
      )}
    </div>
  )
}

/** What the person sees instead of buttons, by variant. */
const NOTE: Partial<Record<ReturnType<typeof approvalView>['kind'], string>> = {
  requester: 'Someone else must approve this run.',
  answered: 'You approved this run.',
  restricted: 'An approver or workspace admin can approve this.',
}

export function ApprovalPanel({ run }: { run: RunInfo }) {
  const me = useSession((s) => s.me)
  const start = useDecision((s) => s.start)
  const approval = run.approval
  if (!approval) return null
  const view = approvalView({ run, me })
  const k = approval.approvals.length
  const n = approval.needed
  const waiting = run.state === 'pending_approval'
  const note = NOTE[view.kind]
  return (
    <section
      aria-labelledby="run-approval-title"
      data-approval-panel={view.kind}
      className="flex min-w-0 flex-col gap-2 border border-border px-3 py-2.5"
    >
      <div className="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h2 id="run-approval-title" className={H2}>
          Approval
        </h2>
        <span data-approval-count={`${String(k)}/${String(n)}`} className="text-sm tabular-nums">
          {k} of {n} {plural(n, 'approval')}
        </span>
      </div>
      <p className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs">
        <span data-approval-policy>
          Needs {n} {plural(n, 'approval')}
          {approval.four_eyes && <span className="text-muted-foreground"> · four-eyes</span>}
        </span>
        <TermTip term="approval-policy" />
      </p>
      <Approvals approval={approval} />
      {approval.denial && <Denial denial={approval.denial} />}
      {waiting && (
        <p data-approval-window className="text-xs text-muted-foreground">
          Window closes <At at={approval.expires_at} /> ({when(approval.expires_at)})
        </p>
      )}
      {note && (
        <p data-approval-note className="text-sm">
          {note}
        </p>
      )}
      {(view.kind === 'act' || view.kind === 'requester') && (
        <div className="flex flex-wrap gap-2">
          {view.kind === 'act' && (
            <Button
              size="sm"
              data-approve
              onClick={() => {
                start('approve', run.id)
              }}
            >
              <FingerprintIcon aria-hidden />
              Approve
            </Button>
          )}
          <Button
            size="sm"
            variant="destructive"
            data-deny
            onClick={() => {
              start('deny', run.id)
            }}
          >
            <ProhibitIcon aria-hidden />
            Deny
          </Button>
        </div>
      )}
    </section>
  )
}

/** A deny's optional reason, kept with the denial and shown to everyone who opens the run. */
function ReasonField() {
  const reason = useDecision((s) => s.reason)
  const setReason = useDecision((s) => s.setReason)
  const busy = useDecision((s) => s.busy)
  const id = useId()
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex items-baseline justify-between gap-2">
        <label htmlFor={id} className={LABEL}>
          Reason (optional)
        </label>
        <span data-reason-count aria-live="polite" className="text-xs text-muted-foreground tabular-nums">
          {reason.length}/{MAX_REASON}
        </span>
      </div>
      <textarea
        id={id}
        className={`${FIELD} h-auto min-h-16 py-1.5`}
        placeholder="Why this run should not go ahead. Everyone who opens the run sees it."
        value={reason}
        maxLength={MAX_REASON}
        disabled={busy}
        onChange={(e) => {
          setReason(e.target.value)
        }}
      />
    </div>
  )
}

/** The preview → confirm dialog for an approval or a denial, with any refusal in the server's words. */
export function DecisionDialog() {
  const kind = useDecision((s) => s.kind)
  const preview = useDecision((s) => s.preview)
  const refusal = useDecision((s) => s.refusal)
  const busy = useDecision((s) => s.busy)
  const confirm = useDecision((s) => s.confirm)
  const close = useDecision((s) => s.close)
  return (
    <Dialog
      open={kind !== null}
      onClose={close}
      data-decision={kind ?? undefined}
      title={kind === 'deny' ? 'Deny this run' : 'Approve this run'}
      description={
        preview ? <span data-preview-summary>{preview.summary}</span> : busy ? 'Getting the preview…' : undefined
      }
      footer={
        <>
          <Button variant="ghost" onClick={close}>
            {refusal ? 'Done' : 'Cancel'}
          </Button>
          {!refusal && (
            <Button
              variant={kind === 'deny' ? 'destructive' : 'default'}
              disabled={busy || !preview}
              onClick={() => {
                void confirm()
              }}
            >
              {preview?.step_up && <FingerprintIcon aria-hidden />}
              Confirm
            </Button>
          )}
        </>
      }
    >
      {preview && <PreviewDetails preview={preview} />}
      {kind === 'deny' && !refusal && <ReasonField />}
      {refusal && <RefusalText refusal={refusal} />}
    </Dialog>
  )
}


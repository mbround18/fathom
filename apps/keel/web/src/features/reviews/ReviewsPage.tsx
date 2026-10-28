/**
 * Reviews (spec 055 US8, T102): signed-mode runs Keel's early check held back, one group of hosts
 * per base. A group shows its commits with their signature status, the files that change trust or
 * targeting (`.keel/`, the inventory, `keel_runner_*` variables), and its diff file by file, page
 * by page. Approving signs the override with a passkey and needs a comment; so does denying.
 *
 * The workspace's trusted signers (the early check's list) are edited at the foot of the page.
 */
import { CaretLeftIcon, CaretRightIcon, PlusIcon, XIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useId, useState } from 'react'

import { GrantWorkspacePicker } from '@/app/WorkspacePicker'
import type { ReviewState, ReviewInfo, SignatureStatus, SignerInfo, SignerKind } from '@/protocol/wire'
import { ConfirmDialog as Confirm } from '@/ui/ConfirmDialog'
import { DataTable } from '@/ui/DataTable'
import { ErrorLine as Alert } from '@/ui/ErrorLine'
import { FIELD, H2, LABEL, LINK, Select } from '@/ui/form'
import { Page } from '@/ui/Page'
import { PageHeader } from '@/ui/PageHeader'

import {
  FLAG_LABELS,
  MAX_COMMENT,
  REASON_LABELS,
  STATUS_LABELS,
  commentOk,
  shortSha,
  useReviews,
} from './store'

function statusTone(status: SignatureStatus): string {
  if (status === 'good') return 'text-status-ok'
  if (status === 'unchecked') return 'text-muted-foreground'
  return 'text-status-err'
}

function stateTone(state: ReviewState): string {
  switch (state) {
    case 'approved':
      return 'text-status-ok'
    case 'denied':
      return 'text-status-err'
    default:
      return 'text-muted-foreground'
  }
}

function ErrorLine() {
  const error = useReviews((s) => s.error)
  const clear = useReviews((s) => s.clearError)
  return <Alert message={error} onDismiss={clear} />
}

/** Picks one of the signed-in user's workspaces and lists its reviews. */
function WorkspacePicker() {
  const workspace = useReviews((s) => s.workspace)
  const load = useReviews((s) => s.load)
  return <GrantWorkspacePicker workspace={workspace} load={load} />
}

function FilterPicker() {
  const filter = useReviews((s) => s.filter)
  const setFilter = useReviews((s) => s.setFilter)
  const id = useId()
  return (
    <div className="flex min-w-0 items-center gap-2">
      <label htmlFor={id} className={LABEL}>
        Show
      </label>
      <Select
        id={id}
        value={filter ?? 'all'}
        onChange={(e) => {
          setFilter(e.target.value === 'all' ? null : (e.target.value as ReviewState))
        }}
      >
        <option value="open">Open</option>
        <option value="approved">Approved</option>
        <option value="denied">Denied</option>
        <option value="expired">Expired</option>
        <option value="all">All</option>
      </Select>
    </div>
  )
}

function ReviewList() {
  const items = useReviews((s) => s.items)
  const selected = useReviews((s) => s.selected)
  const select = useReviews((s) => s.select)
  return (
    <DataTable<ReviewInfo>
      label="Reviews"
      rows={items}
      rowKey={(r) => r.id}
      rowData={(r) => ({ 'data-review-state': r.state })}
      selected={(r) => r.id === selected}
      columns={[
        {
          key: 'job',
          header: 'Job',
          cell: (r) => (
            <button
              type="button"
              className={`${LINK} font-mono break-all`}
              aria-label={`Open review of ${r.job_name} on ${r.hosts.join(', ')}`}
              onClick={() => {
                select(r.id)
              }}
            >
              {r.job_name}
            </button>
          ),
        },
        { key: 'commit', header: 'Commit', cell: (r) => <span className="font-mono">{shortSha(r.target)}</span> },
        { key: 'base', header: 'Base', cell: (r) => <span className="font-mono">{shortSha(r.base)}</span> },
        { key: 'hosts', header: 'Hosts', className: 'w-[18%]', cell: (r) => <span className="font-mono">{r.hosts.join(', ')}</span> },
        { key: 'why', header: 'Why', cell: (r) => REASON_LABELS[r.reason] },
        {
          key: 'state',
          header: 'State',
          cell: (r) => <span className={`text-[11px] tracking-wider uppercase ${stateTone(r.state)}`}>{r.state}</span>,
        },
        {
          key: 'approvals',
          header: 'Approvals',
          cell: (r) => (
            <span className="tabular-nums">
              {r.approvals}/{r.quorum}
            </span>
          ),
        },
        {
          key: 'flagged',
          header: 'Flagged',
          cell: (r) => <span className={`tabular-nums ${r.flagged > 0 ? 'text-status-warn' : ''}`}>{r.flagged}</span>,
        },
      ]}
      empty="Nothing to review. Runs whose commits fail the signature check wait here."
    />
  )
}

function Commits() {
  const detail = useReviews((s) => s.detail)
  if (!detail) return null
  return (
    <section aria-label="Commits" className="flex min-w-0 flex-col gap-1">
      <h3 className={LABEL}>Commits</h3>
      <ul className="flex min-w-0 flex-col border border-border text-xs">
        {detail.commits.map((c) => (
          <li
            key={c.id}
            data-commit={c.id}
            data-signature={c.status}
            className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5 border-t border-border px-2 py-1 first:border-t-0"
          >
            <span className="font-mono">{shortSha(c.id)}</span>
            <span className={statusTone(c.status)}>{STATUS_LABELS[c.status]}</span>
            {c.signer && <span className="font-mono text-muted-foreground">{c.signer}</span>}
            <span className="min-w-0 flex-1 truncate">{c.summary}</span>
            <span className="text-muted-foreground">{c.author}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

function Flags() {
  const flags = useReviews((s) => s.detail?.flags ?? [])
  if (flags.length === 0) return null
  return (
    <section aria-label="Flagged changes" className="flex min-w-0 flex-col gap-1 border border-status-warn/50 bg-status-warn/5 p-2">
      <h3 className="text-[11px] tracking-wider text-status-warn uppercase">These change who or what Keel trusts and targets</h3>
      <ul className="flex min-w-0 flex-col gap-1 text-xs">
        {flags.map((f) => (
          <li key={`${f.kind}:${f.path}`} data-flag={f.kind}>
            <span className="text-status-warn">{FLAG_LABELS[f.kind]}</span>{' '}
            <span className="font-mono break-all">{f.path}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

function lineTone(line: string): string {
  if (line.startsWith('@@')) return 'text-muted-foreground'
  if (line.startsWith('+')) return 'text-status-ok'
  if (line.startsWith('-')) return 'text-status-err'
  return ''
}

function Diff() {
  const files = useReviews((s) => s.files)
  const diff = useReviews((s) => s.diff)
  const openFile = useReviews((s) => s.openFile)
  if (files.length === 0) return null
  return (
    <section aria-label="Diff" className="flex min-w-0 flex-col gap-2">
      <h3 className={LABEL}>Changed files</h3>
      <ul aria-label="Changed files" className="flex min-w-0 flex-col gap-1 text-xs">
        {files.map((f) => (
          <li key={f.path} className="flex min-w-0 flex-wrap items-center gap-2">
            <Button
              variant={diff?.path === f.path ? 'outline' : 'ghost'}
              size="xs"
              className="h-auto min-h-6 max-w-full font-mono break-all whitespace-normal normal-case"
              aria-current={diff?.path === f.path ? 'true' : undefined}
              onClick={() => {
                openFile(f.path)
              }}
            >
              {f.path}
            </Button>
            <span className="text-muted-foreground">{f.status}</span>
            {f.flags.map((k) => (
              <span key={k} data-file-flag={k} className="text-status-warn">
                {FLAG_LABELS[k]}
              </span>
            ))}
          </li>
        ))}
      </ul>
      {diff && (
        <div aria-label={`Diff of ${diff.path}`} role="region" className="flex min-w-0 flex-col gap-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2 text-xs">
            <span className="min-w-0 font-mono break-all">{diff.path}</span>
            <span className="text-muted-foreground" data-diff-page>
              Page {diff.page + 1} of {Math.max(diff.pages, 1)}
            </span>
            <Button
              variant="outline"
              size="xs"
              disabled={diff.page <= 0}
              onClick={() => {
                openFile(diff.path, diff.page - 1)
              }}
            >
              <CaretLeftIcon aria-hidden />
              Previous page
            </Button>
            <Button
              variant="outline"
              size="xs"
              disabled={diff.page + 1 >= diff.pages}
              onClick={() => {
                openFile(diff.path, diff.page + 1)
              }}
            >
              Next page
              <CaretRightIcon aria-hidden />
            </Button>
          </div>
          {diff.binary ? (
            <p className="text-xs text-muted-foreground">Binary file: no text diff.</p>
          ) : (
            <pre className="max-h-[60vh] overflow-auto border border-border bg-background p-2 font-mono text-[11px]">
              {diff.lines.map((l, i) => (
                <div key={i} className={lineTone(l)}>
                  {l === '' ? ' ' : l}
                </div>
              ))}
            </pre>
          )}
        </div>
      )}
    </section>
  )
}

function Decisions() {
  const decisions = useReviews((s) => s.detail?.decisions ?? [])
  if (decisions.length === 0) return null
  return (
    <section aria-label="Decisions" className="flex min-w-0 flex-col gap-1">
      <h3 className={LABEL}>Decisions</h3>
      <ul className="flex min-w-0 flex-col border border-border text-xs">
        {decisions.map((d) => (
          <li key={`${d.approver}:${d.at}`} className="border-t border-border px-2 py-1 first:border-t-0">
            <span className={d.approve ? 'text-status-ok' : 'text-status-err'}>{d.approve ? 'Approved' : 'Denied'}</span>{' '}
            by {d.approver} at {d.at}
            {d.expires_at && <span className="text-muted-foreground"> (lapses {d.expires_at})</span>}
            {d.comment && <p className="text-muted-foreground">{d.comment}</p>}
          </li>
        ))}
      </ul>
    </section>
  )
}

/** The comment and the approve and deny buttons, for an open group. */
function DecideForm() {
  const review = useReviews((s) => s.detail?.review)
  const approve = useReviews((s) => s.approve)
  const deny = useReviews((s) => s.deny)
  const [comment, setComment] = useState('')
  const id = useId()
  if (review?.state !== 'open') return null
  const ok = commentOk(comment)
  return (
    <form
      aria-label="Decide"
      className="flex min-w-0 flex-col gap-2 border-t border-border pt-3"
      onSubmit={(e) => {
        e.preventDefault()
        approve(comment)
      }}
    >
      <label htmlFor={id} className={LABEL}>
        Comment (required)
      </label>
      <textarea
        id={id}
        className={`${FIELD} h-auto min-h-16 py-1.5`}
        placeholder="What you checked, and why it is safe (or not) to run."
        value={comment}
        maxLength={MAX_COMMENT}
        onChange={(e) => {
          setComment(e.target.value)
        }}
      />
      {review.four_eyes && (
        <p className="text-xs text-muted-foreground">
          Four-eyes: someone other than whoever asked for the run or wrote these commits must approve.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm" disabled={!ok}>
          Approve with passkey
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!ok}
          onClick={() => {
            deny(comment)
          }}
        >
          Deny
        </Button>
      </div>
    </form>
  )
}

function Detail() {
  const detail = useReviews((s) => s.detail)
  const select = useReviews((s) => s.select)
  if (!detail) return null
  const r = detail.review
  return (
    <section aria-label="Review" data-review={r.id} className="flex min-w-0 flex-col border border-border">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center justify-between gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <h2 className="min-w-0 text-xs font-semibold break-words">
          {r.job_name} at <span className="font-mono">{shortSha(r.target)}</span> on{' '}
          <span className="font-mono">{r.hosts.join(', ')}</span>
        </h2>
        <Button
          variant="ghost"
          size="xs"
          onClick={() => {
            select(null)
          }}
        >
          <XIcon aria-hidden />
          Close
        </Button>
      </div>
      <div className="flex min-w-0 flex-col gap-4 p-3">
        <p className="text-xs text-muted-foreground">
          {REASON_LABELS[r.reason]}. Base <span className="font-mono">{shortSha(r.base)}</span>.{' '}
          <span className={`tracking-wider uppercase ${stateTone(r.state)}`} data-state>
            {r.state}
          </span>{' '}
          ({r.approvals}/{r.quorum} approvals)
        </p>
        <Commits />
        <Flags />
        <Diff />
        <Decisions />
        <DecideForm key={r.id} />
      </div>
    </section>
  )
}

/** Confirms an override (signed with a passkey), a denial, or a signer change. */
function ConfirmDialog() {
  const pending = useReviews((s) => s.pending)
  const signing = useReviews((s) => s.signing)
  const confirm = useReviews((s) => s.confirm)
  const cancel = useReviews((s) => s.cancel)
  const approving = pending?.kind === 'approve'
  const waiting = approving && pending.challenge === null
  return (
    <Confirm
      // The approve dialog explains the passkey itself, with what it signs.
      preview={pending && (approving ? { ...pending.preview, step_up: false } : pending.preview)}
      title={approving ? 'Approve with passkey' : 'Confirm'}
      confirmLabel={approving ? 'Sign and approve' : 'Confirm'}
      busy={waiting || signing}
      onCancel={cancel}
      onConfirm={() => void confirm()}
    >
      {pending?.kind === 'approve' && pending.challenge && (
        <div className="flex min-w-0 flex-col gap-1 text-xs">
          <p>
            Your passkey signs exactly this. Hosts check the signature before they run, and the override lapses at{' '}
            <span data-override-expires>{pending.challenge.expires_at}</span>.
          </p>
          <pre
            data-canonical
            className="max-h-40 overflow-auto border border-border bg-background p-2 font-mono text-[11px] break-all whitespace-pre-wrap"
          >
            {pending.challenge.canonical}
          </pre>
        </div>
      )}
    </Confirm>
  )
}

/** The workspace's trusted signers: the early check's list. */
function Signers() {
  const signers = useReviews((s) => s.signers)
  const addSigner = useReviews((s) => s.addSigner)
  const removeSigner = useReviews((s) => s.removeSigner)
  const [kind, setKind] = useState<SignerKind>('ssh')
  const [key, setKey] = useState('')
  const [label, setLabel] = useState('')
  const kindId = useId()
  const keyId = useId()
  const labelId = useId()
  const ok = key.trim().length > 0 && label.trim().length > 0
  return (
    <section aria-labelledby="signers-title" className="flex min-w-0 flex-col gap-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <h2 id="signers-title" className={H2}>
          Trusted signers
        </h2>
        <p className="max-w-prose text-xs text-muted-foreground">
        Commits signed by these keys pass Keel&apos;s early check. Hosts check signatures against their own
        keel_runner_trusted_signers, so keep the two lists in step.
        </p>
      </div>
      <DataTable<SignerInfo>
        label="Trusted signers"
        rows={signers}
        rowKey={(s) => s.id}
        columns={[
          { key: 'label', header: 'Label', cell: (s) => s.label },
          { key: 'kind', header: 'Kind', cell: (s) => (s.kind === 'ssh' ? 'SSH' : 'OpenPGP') },
          { key: 'fp', header: 'Fingerprint', className: 'w-[40%]', cell: (s) => <span className="font-mono break-all">{s.fingerprint}</span> },
          { key: 'added', header: 'Added', cell: (s) => <span className="tabular-nums">{s.added_at}</span> },
        ]}
        actions={(s) => (
          <Button
            variant="ghost"
            size="xs"
            className="text-status-err"
            aria-label={`Remove signer ${s.label}`}
            onClick={() => {
              removeSigner(s.id)
            }}
          >
            Remove
          </Button>
        )}
        empty="No trusted signers: every commit needs a review."
      />
      <form
        aria-label="Add a signer"
        className="flex min-w-0 flex-wrap items-end gap-2 border border-border bg-muted/20 p-3"
        onSubmit={(e) => {
          e.preventDefault()
          if (ok) addSigner(kind, key.trim(), label.trim())
        }}
      >
        <div className="flex min-w-0 flex-col gap-1">
          <label htmlFor={kindId} className={LABEL}>
            Kind
          </label>
          <Select
            id={kindId}
            value={kind}
            onChange={(e) => {
              setKind(e.target.value as SignerKind)
            }}
          >
            <option value="ssh">SSH</option>
            <option value="openpgp">OpenPGP</option>
          </Select>
        </div>
        <div className="flex min-w-0 flex-1 basis-60 flex-col gap-1">
          <label htmlFor={keyId} className={LABEL}>
            Public key
          </label>
          <textarea
            id={keyId}
            className={`${FIELD} h-auto min-h-8 py-1.5 font-mono`}
            value={key}
            placeholder={kind === 'ssh' ? 'ssh-ed25519 AAAA… ci@example.org' : '-----BEGIN PGP PUBLIC KEY BLOCK-----'}
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="flex min-w-0 flex-col gap-1">
          <label htmlFor={labelId} className={LABEL}>
            Label
          </label>
          <input
            id={labelId}
            className={FIELD}
            value={label}
            autoComplete="off"
            onChange={(e) => {
              setLabel(e.target.value)
            }}
          />
        </div>
        <Button type="submit" size="sm" disabled={!ok}>
          <PlusIcon aria-hidden />
          Add signer
        </Button>
      </form>
    </section>
  )
}

export default function ReviewsPage() {
  const workspace = useReviews((s) => s.workspace)
  return (
    <Page name="reviews" labelledBy="reviews-title">
      <PageHeader
        id="reviews-title"
        title="Reviews"
        description="Runs held for a person to check, because their commits were not signed by a trusted key."
      >
        <WorkspacePicker />
        {workspace && <FilterPicker />}
      </PageHeader>
      <ErrorLine />
      {workspace && (
        <>
          <ReviewList />
          <Detail />
          <Signers />
        </>
      )}
      <ConfirmDialog />
    </Page>
  )
}

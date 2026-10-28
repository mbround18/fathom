/**
 * Reviews (spec 055 US8, T102): signed-mode runs Keel's early check held back, one group of hosts
 * per base. A group shows its commits with their signature status, the files that change trust or
 * targeting (`.keel/`, the inventory, `keel_runner_*` variables), and its diff file by file, page
 * by page. Approving signs the override with a passkey and needs a comment; so does denying.
 *
 * The workspace's trusted signers (the early check's list) are edited at the foot of the page.
 */
import { Button, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@companion/ui'
import { useEffect, useId, useState } from 'react'

import { useSession } from '@/app/session-store'
import type { ReviewState, SignatureStatus, SignerKind } from '@/protocol/wire'

import {
  FIELD,
  FLAG_LABELS,
  MAX_COMMENT,
  REASON_LABELS,
  STATUS_LABELS,
  commentOk,
  shortSha,
  useReviews,
} from './store'

const LABEL = 'text-[11px] tracking-wider text-muted-foreground uppercase'

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
  if (!error) return null
  return (
    <p role="alert" className="flex items-center justify-between gap-2 border border-status-err/40 px-2 py-1 text-xs text-status-err">
      <span>{error}</span>
      <Button variant="ghost" size="xs" onClick={clear}>
        Dismiss
      </Button>
    </p>
  )
}

/** Picks one of the signed-in user's workspaces and lists its reviews. */
function WorkspacePicker() {
  const grants = useSession((s) => s.me?.grants ?? [])
  const workspace = useReviews((s) => s.workspace)
  const load = useReviews((s) => s.load)
  const id = useId()
  const first = grants[0]?.workspace ?? null
  const unique = [...new Map(grants.map((g) => [g.workspace, g.workspace_slug])).entries()]

  useEffect(() => {
    const current = workspace ?? first
    if (current) load(current)
  }, [workspace, first, load])

  if (unique.length === 0)
    return (
      <p data-pane-state="empty" className="text-xs text-muted-foreground">
        You have no workspace yet. An owner or admin grants one from Teams.
      </p>
    )
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className={LABEL}>
        Workspace
      </label>
      <select
        id={id}
        className={FIELD}
        value={workspace ?? first ?? ''}
        onChange={(e) => {
          load(e.target.value)
        }}
      >
        {unique.map(([ws, slug]) => (
          <option key={ws} value={ws}>
            {slug}
          </option>
        ))}
      </select>
    </div>
  )
}

function FilterPicker() {
  const filter = useReviews((s) => s.filter)
  const setFilter = useReviews((s) => s.setFilter)
  const id = useId()
  return (
    <div className="flex items-center gap-2">
      <label htmlFor={id} className={LABEL}>
        Show
      </label>
      <select
        id={id}
        className={FIELD}
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
      </select>
    </div>
  )
}

function ReviewList() {
  const items = useReviews((s) => s.items)
  const selected = useReviews((s) => s.selected)
  const select = useReviews((s) => s.select)
  return (
    <div className="overflow-x-auto">
      <table aria-label="Reviews" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Job</th>
            <th>Commit</th>
            <th>Base</th>
            <th>Hosts</th>
            <th>Why</th>
            <th>State</th>
            <th>Approvals</th>
            <th>Flagged</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && (
            <tr>
              <td colSpan={8} data-pane-state="empty" className="py-1 text-muted-foreground">
                Nothing to review.
              </td>
            </tr>
          )}
          {items.map((r) => (
            <tr
              key={r.id}
              data-review-state={r.state}
              aria-selected={r.id === selected}
              className={`border-t border-border ${r.id === selected ? 'bg-accent/30' : ''}`}
            >
              <td className="py-1">
                <Button
                  variant="link"
                  size="xs"
                  className="px-0 font-mono"
                  aria-label={`Open review of ${r.job_name} on ${r.hosts.join(', ')}`}
                  onClick={() => {
                    select(r.id)
                  }}
                >
                  {r.job_name}
                </Button>
              </td>
              <td className="font-mono">{shortSha(r.target)}</td>
              <td className="font-mono">{shortSha(r.base)}</td>
              <td className="font-mono">{r.hosts.join(', ')}</td>
              <td>{REASON_LABELS[r.reason]}</td>
              <td className={`uppercase ${stateTone(r.state)}`}>{r.state}</td>
              <td>
                {r.approvals}/{r.quorum}
              </td>
              <td className={r.flagged > 0 ? 'text-status-warn' : ''}>{r.flagged}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function Commits() {
  const detail = useReviews((s) => s.detail)
  if (!detail) return null
  return (
    <section aria-label="Commits" className="flex flex-col gap-1">
      <h3 className={LABEL}>Commits</h3>
      <ul className="flex flex-col gap-1 text-xs">
        {detail.commits.map((c) => (
          <li key={c.id} data-commit={c.id} data-signature={c.status} className="flex flex-wrap items-baseline gap-2">
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
    <section aria-label="Flagged changes" className="flex flex-col gap-1 border border-status-warn/40 p-2">
      <h3 className={LABEL}>These change who or what Keel trusts and targets</h3>
      <ul className="flex flex-col gap-1 text-xs">
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
    <section aria-label="Diff" className="flex flex-col gap-2">
      <h3 className={LABEL}>Changed files</h3>
      <ul aria-label="Changed files" className="flex flex-col gap-1 text-xs">
        {files.map((f) => (
          <li key={f.path} className="flex flex-wrap items-center gap-2">
            <Button
              variant={diff?.path === f.path ? 'outline' : 'ghost'}
              size="xs"
              className="font-mono"
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
        <div aria-label={`Diff of ${diff.path}`} role="region" className="flex flex-col gap-1">
          <div className="flex items-center gap-2 text-xs">
            <span className="font-mono">{diff.path}</span>
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
    <section aria-label="Decisions" className="flex flex-col gap-1">
      <h3 className={LABEL}>Decisions</h3>
      <ul className="flex flex-col gap-1 text-xs">
        {decisions.map((d) => (
          <li key={`${d.approver}:${d.at}`}>
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
      className="flex flex-col gap-2"
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
        className="min-h-16 border border-input bg-background p-2 text-xs outline-none focus-visible:border-accent-neon"
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
      <div className="flex gap-2">
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
    <section aria-label="Review" data-review={r.id} className="flex flex-col gap-3 border border-border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-xs font-semibold">
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
          Close
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {REASON_LABELS[r.reason]}. Base <span className="font-mono">{shortSha(r.base)}</span>.{' '}
        <span className={`uppercase ${stateTone(r.state)}`} data-state>
          {r.state}
        </span>{' '}
        ({r.approvals}/{r.quorum} approvals)
      </p>
      <Commits />
      <Flags />
      <Diff />
      <Decisions />
      <DecideForm key={r.id} />
    </section>
  )
}

/** Confirms an override (signed with a passkey), a denial, or a signer change. */
function ConfirmDialog() {
  const pending = useReviews((s) => s.pending)
  const signing = useReviews((s) => s.signing)
  const confirm = useReviews((s) => s.confirm)
  const cancel = useReviews((s) => s.cancel)
  const waiting = pending?.kind === 'approve' && pending.challenge === null
  return (
    <Dialog
      open={pending !== null}
      onOpenChange={(open) => {
        if (!open) cancel()
      }}
    >
      {pending && (
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{pending.kind === 'approve' ? 'Approve with passkey' : 'Confirm'}</DialogTitle>
            <DialogDescription data-preview-summary>{pending.preview.summary}</DialogDescription>
          </DialogHeader>
          {pending.preview.affected.length > 0 && (
            <ul aria-label="Affected" className="flex flex-col gap-1 text-xs">
              {pending.preview.affected.map((a) => (
                <li key={a} className="font-mono break-all">
                  {a}
                </li>
              ))}
            </ul>
          )}
          {pending.kind === 'approve' && pending.challenge && (
            <div className="flex flex-col gap-1 text-xs">
              <p>
                Your passkey signs exactly this. Hosts check the signature before they run, and the override lapses at{' '}
                <span data-override-expires>{pending.challenge.expires_at}</span>.
              </p>
              <pre data-canonical className="max-h-40 overflow-auto border border-border bg-background p-2 font-mono text-[11px] break-all whitespace-pre-wrap">
                {pending.challenge.canonical}
              </pre>
            </div>
          )}
          {pending.kind !== 'approve' && pending.preview.step_up && (
            <p className="text-xs text-muted-foreground">This change asks for your passkey.</p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={cancel}>
              Cancel
            </Button>
            <Button disabled={waiting || signing} onClick={() => void confirm()}>
              {pending.kind === 'approve' ? 'Sign and approve' : 'Confirm'}
            </Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
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
    <section aria-labelledby="signers-title" className="flex flex-col gap-2 border-t border-border pt-3">
      <h2 id="signers-title" className="font-display text-xs font-semibold tracking-wider uppercase">
        Trusted signers
      </h2>
      <p className="text-xs text-muted-foreground">
        Commits signed by these keys pass Keel&apos;s early check. Hosts check signatures against their own
        keel_runner_trusted_signers, so keep the two lists in step.
      </p>
      <table aria-label="Trusted signers" className="w-full text-left text-xs">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1">Label</th>
            <th>Kind</th>
            <th>Fingerprint</th>
            <th>Added</th>
            <th>
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {signers.length === 0 && (
            <tr>
              <td colSpan={5} data-pane-state="empty" className="py-1 text-muted-foreground">
                No trusted signers: every commit needs a review.
              </td>
            </tr>
          )}
          {signers.map((s) => (
            <tr key={s.id} className="border-t border-border">
              <td className="py-1">{s.label}</td>
              <td>{s.kind}</td>
              <td className="font-mono break-all">{s.fingerprint}</td>
              <td className="whitespace-nowrap">{s.added_at}</td>
              <td className="text-right">
                <Button
                  variant="ghost"
                  size="xs"
                  aria-label={`Remove signer ${s.label}`}
                  onClick={() => {
                    removeSigner(s.id)
                  }}
                >
                  Remove
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <form
        aria-label="Add a signer"
        className="flex flex-wrap items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          if (ok) addSigner(kind, key.trim(), label.trim())
        }}
      >
        <div className="flex flex-col gap-1">
          <label htmlFor={kindId} className={LABEL}>
            Kind
          </label>
          <select
            id={kindId}
            className={FIELD}
            value={kind}
            onChange={(e) => {
              setKind(e.target.value as SignerKind)
            }}
          >
            <option value="ssh">SSH</option>
            <option value="openpgp">OpenPGP</option>
          </select>
        </div>
        <div className="flex min-w-60 flex-1 flex-col gap-1">
          <label htmlFor={keyId} className={LABEL}>
            Public key
          </label>
          <textarea
            id={keyId}
            className="min-h-8 border border-input bg-background p-2 font-mono text-xs outline-none focus-visible:border-accent-neon"
            value={key}
            placeholder={kind === 'ssh' ? 'ssh-ed25519 AAAA… ci@example.org' : '-----BEGIN PGP PUBLIC KEY BLOCK-----'}
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="flex flex-col gap-1">
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
          Add signer
        </Button>
      </form>
    </section>
  )
}

export default function ReviewsPage() {
  const workspace = useReviews((s) => s.workspace)
  return (
    <section data-page="reviews" aria-labelledby="reviews-title" className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 id="reviews-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Reviews
        </h1>
        <WorkspacePicker />
        {workspace && <FilterPicker />}
      </div>
      <ErrorLine />
      {workspace && (
        <>
          <ReviewList />
          <Detail />
          <Signers />
        </>
      )}
      <ConfirmDialog />
    </section>
  )
}

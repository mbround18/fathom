/**
 * Hosts → Join requests (spec 055 T148, contracts/join.md): hosts that knocked with the workspace's
 * join token and wait for an operator. Loaded when the tab is first opened.
 *
 * Only the key fingerprint is proof of anything: it is shown in full beside the instruction to
 * compare it with `keel-runner fingerprint` on the host. Everything else is the host's claim, and
 * labelled so. Accept is prefilled with the host's suggested name and the token's profiles; Keel
 * refuses profiles outside the host's own allowed list, and so does the form. Reject takes an
 * optional note. Both go through the confirm dialog; accept asks for a passkey.
 */
import { CheckIcon, ProhibitIcon, TerminalWindowIcon } from '@phosphor-icons/react'
import { Button, Checkbox } from '@companion/ui'
import {
  ChipList,
  CopyBlock,
  DetailList,
  DetailRow,
  EmptyState,
  Field,
  FIELD,
  FIELD_AREA,
  LoadFailed,
  LoadState,
  SectionIntro,
  Select,
  STATUS,
  Time,
} from '@companion/ui/kit'
import { useEffect, useId, useRef, useState } from 'react'

import { useSession } from '@/app/session-store'
import { useConverge } from '@/features/converge/store'
import { useGovernance } from '@/features/teams/store'
import type { JoinRequestInfo, JoinRequestState } from '@/protocol/wire'

import { HOST_NAME, MAX_PROFILES } from './enroll-store'
import { acceptPrefill, outsideAllowed, reasonText } from './join'
import { useJoinRequests } from './join-store'
import { parseProfiles } from './store'
import { canOperate } from './workspaces'

const STATES: readonly JoinRequestState[] = ['pending', 'accepted', 'rejected', 'expired']

const TONE: Record<JoinRequestState, string> = {
  pending: 'text-status-warn',
  accepted: 'text-status-ok',
  rejected: 'text-status-err',
  expired: 'text-muted-foreground',
}

function StateFilter() {
  const state = useJoinRequests((s) => s.state)
  const setState = useJoinRequests((s) => s.setState)
  const id = useId()
  return (
    <div className="flex min-w-0 items-center gap-2 text-xs">
      <label htmlFor={id} className="text-muted-foreground">
        State
      </label>
      <Select
        id={id}
        dense
        value={state ?? ''}
        onChange={(e) => {
          setState(STATES.find((s) => s === e.target.value) ?? null)
        }}
      >
        {STATES.map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
        <option value="">all</option>
      </Select>
    </div>
  )
}

/** The button that opened a form is gone once it opens: focus its first field instead. */
function useOpenFocus<T extends HTMLElement>() {
  const ref = useRef<T>(null)
  useEffect(() => {
    ref.current?.focus()
  }, [])
  return ref
}

/** Accept: inventory name and profiles, prefilled; profiles outside the host's list are refused. */
function AcceptForm({ req, onDone }: { req: JoinRequestInfo; onDone: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const prefill = acceptPrefill(req)
  const [name, setName] = useState(prefill.inventoryName)
  const [profiles, setProfiles] = useState(prefill.profiles.join(', '))
  const [converge, setConverge] = useState(true)
  const convergeOff = useConverge((s) => s.disabled)
  const nameId = useId()
  const profilesId = useId()
  const convergeId = useId()
  const picked = parseProfiles(profiles)
  const outside = outsideAllowed(picked, req.allowed_profiles)
  const first = useOpenFocus<HTMLInputElement>()
  const nameOk = HOST_NAME.test(name)
  const profilesOk = picked.length > 0 && picked.length <= MAX_PROFILES && outside.length === 0
  return (
    <form
      aria-label="Accept join request"
      className="flex min-w-0 flex-col gap-3 border-t border-border bg-muted/20 p-3"
      onSubmit={(e) => {
        e.preventDefault()
        if (!nameOk || !profilesOk) return
        propose(
          `Accept ${name}`,
          {
            type: 'PreviewJoinAccept',
            // Left out means yes, so the default stays what older Keel sent.
            data:
              convergeOff || converge
                ? { request: req.id, inventory_name: name, profiles: picked }
                : { request: req.id, inventory_name: name, profiles: picked, auto_converge: false },
          },
          (event) => {
            if (event.type === 'JoinRequestDecided') onDone()
          },
        )
      }}
    >
      <div className="grid min-w-0 gap-3 sm:grid-cols-2">
        <Field label="Inventory name" htmlFor={nameId} hint={nameOk ? undefined : 'Letters, digits, dots, dashes and underscores; starts with a letter or digit.'}>
          <input
            id={nameId}
            ref={first}
            className={`${FIELD} font-mono`}
            value={name}
            aria-invalid={!nameOk}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => {
              setName(e.target.value.trim())
            }}
          />
        </Field>
        <Field
          label="Profiles"
          htmlFor={profilesId}
          hint={
            outside.length > 0
              ? `The host does not allow ${outside.join(', ')}.`
              : picked.length === 0
                ? `Name at least one. The host allows ${req.allowed_profiles.join(', ') || 'none'}.`
                : `The host allows ${req.allowed_profiles.join(', ') || 'none'}.`
          }
        >
          <input
            id={profilesId}
            className={`${FIELD} font-mono`}
            value={profiles}
            aria-invalid={!profilesOk}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => {
              setProfiles(e.target.value)
            }}
          />
        </Field>
      </div>
      {!convergeOff && (
        <div data-auto-converge={converge} className="flex min-w-0 items-start gap-2">
          <Checkbox
            id={convergeId}
            className="mt-0.5"
            checked={converge}
            aria-describedby={`${convergeId}-hint`}
            onCheckedChange={(c) => {
              setConverge(c === true)
            }}
          />
          <div className="flex min-w-0 flex-col gap-0.5">
            <label htmlFor={convergeId} className="text-xs font-medium">
              Converge after accepting
            </label>
            <p id={`${convergeId}-hint`} className="text-xs break-words text-muted-foreground">
              {converge
                ? picked.length > 0
                  ? `Once accepted, Keel runs the steps for ${picked.join(', ')} on it, through the approval policy.`
                  : 'Once accepted, Keel runs the steps for its profiles on it, through the approval policy.'
                : 'Keel waits for someone to converge it.'}
            </p>
          </div>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm" disabled={!nameOk || !profilesOk}>
          <CheckIcon aria-hidden />
          Accept
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  )
}

/** Reject, with an optional note kept on the request. */
function RejectForm({ req, onDone }: { req: JoinRequestInfo; onDone: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const [comment, setComment] = useState('')
  const id = useId()
  const tooLong = comment.length > 500
  const first = useOpenFocus<HTMLTextAreaElement>()
  return (
    <form
      aria-label="Reject join request"
      className="flex min-w-0 flex-col gap-3 border-t border-border bg-muted/20 p-3"
      onSubmit={(e) => {
        e.preventDefault()
        if (tooLong) return
        const note = comment.trim()
        propose(
          `Reject ${req.hostname || req.inventory_name}`,
          { type: 'PreviewJoinReject', data: { request: req.id, comment: note.length > 0 ? note : null } },
          (event) => {
            if (event.type === 'JoinRequestDecided') onDone()
          },
        )
      }}
    >
      <Field
        label="Note (optional)"
        htmlFor={id}
        hint={tooLong ? 'At most 500 characters.' : 'Kept on the request. The host is refused for 30 days and stops asking.'}
      >
        <textarea
          id={id}
          ref={first}
          className={`${FIELD_AREA} max-w-prose`}
          value={comment}
          aria-invalid={tooLong}
          onChange={(e) => {
            setComment(e.target.value)
          }}
        />
      </Field>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm" variant="destructive" disabled={tooLong}>
          <ProhibitIcon aria-hidden />
          Reject
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  )
}

function Request({ req, canAct }: { req: JoinRequestInfo; canAct: boolean }) {
  const [open, setOpen] = useState<'accept' | 'reject' | null>(null)
  const title = req.hostname || req.inventory_name
  const reason = reasonText(req.reason)
  return (
    <li data-join-request={req.id} data-join-state={req.state} className="flex min-w-0 flex-col border border-border">
      <div className="flex min-h-10 min-w-0 flex-wrap items-center gap-2 border-b border-border bg-muted/20 px-3 py-1.5">
        <h2 className="min-w-0 font-mono text-sm break-all">{title}</h2>
        <span className={`${STATUS} ${TONE[req.state]}`}>{req.state}</span>
        <span className="text-xs text-muted-foreground">
          last asked <Time at={req.last_seen_at} relative />
        </span>
        {canAct && req.state === 'pending' && open === null && (
          <div className="ml-auto flex gap-1">
            <Button
              size="xs"
              aria-label={`Accept ${title}`}
              onClick={() => {
                setOpen('accept')
              }}
            >
              <CheckIcon aria-hidden />
              Accept
            </Button>
            <Button
              size="xs"
              variant="ghost"
              className="text-status-err"
              aria-label={`Reject ${title}`}
              onClick={() => {
                setOpen('reject')
              }}
            >
              <ProhibitIcon aria-hidden />
              Reject
            </Button>
          </div>
        )}
      </div>
      <div className="flex min-w-0 flex-col gap-3 p-3">
        <div className="flex min-w-0 flex-col gap-1">
          <CopyBlock label="Host key fingerprint" value={req.fingerprint} data="join-fingerprint" />
          <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
            <TerminalWindowIcon aria-hidden className="mt-0.5 shrink-0" />
            <span>
              Run <code className="font-mono text-foreground">keel-runner fingerprint</code> on the host and accept only if it
              prints the same value. Nothing else below is proof of which machine this is.
            </span>
          </p>
        </div>
        <DetailList label="What the host says" columns={2}>
          <DetailRow term="Host says its name is" data="hostname">
            <span className="font-mono break-all">{req.hostname || '—'}</span>
          </DetailRow>
          <DetailRow term="Suggested inventory name" data="inventory-name">
            <span className="font-mono break-all">{req.inventory_name}</span>
          </DetailRow>
          <DetailRow term="OS" data="os">
            {req.os || '—'}
          </DetailRow>
          <DetailRow term="Architecture" data="arch">
            <span className="font-mono">{req.arch}</span>
          </DetailRow>
          <DetailRow term="Runner" data="runner-version">
            <span className="font-mono">{req.runner_version}</span>
          </DetailRow>
          <DetailRow term="Signed jobs only" data="require-signed">
            {req.require_signed ? 'yes' : 'no'}
          </DetailRow>
          <DetailRow term="Allows profiles" data="allowed-profiles">
            <ChipList label="Allowed profiles" items={req.allowed_profiles} />
          </DetailRow>
          <DetailRow term="Came from" data="source">
            <span className="font-mono break-all">{req.source}</span>
          </DetailRow>
          <DetailRow term="First asked" data="first-seen">
            <Time at={req.first_seen_at} />
          </DetailRow>
          <DetailRow term="Token" data="jti">
            <span className="font-mono break-all text-muted-foreground">{req.jti}</span>
          </DetailRow>
          {reason !== null && (
            <DetailRow term="Why" data="reason">
              {reason}
            </DetailRow>
          )}
          {req.comment !== null && (
            <DetailRow term="Note" data="comment">
              <span className="break-words whitespace-pre-wrap">{req.comment}</span>
            </DetailRow>
          )}
        </DetailList>
      </div>
      {open === 'accept' && (
        <AcceptForm
          req={req}
          onDone={() => {
            setOpen(null)
          }}
        />
      )}
      {open === 'reject' && (
        <RejectForm
          req={req}
          onDone={() => {
            setOpen(null)
          }}
        />
      )}
    </li>
  )
}

export default function JoinRequestsTab({ workspace }: { workspace: string }) {
  const items = useJoinRequests((s) => s.items)
  const loaded = useJoinRequests((s) => s.loaded)
  const failed = useJoinRequests((s) => s.failed)
  const state = useJoinRequests((s) => s.state)
  const load = useJoinRequests((s) => s.load)
  const me = useSession((s) => s.me)
  const canAct = canOperate(me, workspace)

  useEffect(() => {
    load(workspace)
  }, [workspace, load])

  return (
    <section aria-label="Join requests" className="flex min-w-0 flex-col gap-3">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <SectionIntro guide={{ guide: 'hosts', anchor: 'join-requests' }}>
          Hosts that asked to join with this workspace&apos;s join token. The token lives in Settings → Hosts.
        </SectionIntro>
        <StateFilter />
      </div>
      {failed ? (
        <LoadFailed
          what="join requests"
          refusal={failed}
          onRetry={() => {
            load(workspace)
          }}
        />
      ) : !loaded ? (
        <LoadState loading label="Loading join requests…" />
      ) : items.length === 0 ? (
        <EmptyState title={state === 'pending' ? 'No host is waiting' : 'No join requests'}>
          {state === 'pending'
            ? 'A host that runs the base profile with the join token shows up here, and keeps asking until you decide.'
            : 'None in this state.'}
        </EmptyState>
      ) : (
        <ul aria-label="Join requests" className="flex min-w-0 flex-col gap-3">
          {items.map((r) => (
            <Request key={r.id} req={r} canAct={canAct} />
          ))}
        </ul>
      )}
    </section>
  )
}

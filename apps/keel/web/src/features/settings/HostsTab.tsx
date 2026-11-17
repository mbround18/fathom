/**
 * Settings → Hosts (spec 055 T148, contracts/join.md "Issuing"): the current workspace's join
 * token. Loaded when the tab is first opened.
 *
 * A workspace has at most one join token. Issue, rotate and disable go through the confirm dialog
 * (they ask for a passkey). The token itself comes back once, in the confirm's `JoinTokenSecret`,
 * and is kept in this component's state only until Done: Keel stores its hash and never shows it
 * again. Beside it, the claims decoded from it say what a host will be told, so the person copying
 * it can check the server and host CA before it goes into Ansible. Rotating the signing key (owners
 * and admins) retires every workspace's token.
 */
import { ArrowsClockwiseIcon, KeyIcon, PlusIcon, ProhibitIcon, WarningIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import {
  CopyBlock,
  DetailList,
  DetailRow,
  Field,
  FIELD,
  H2,
  LoadState,
  SectionIntro,
  Select,
  STATUS,
  Time,
} from '@companion/ui/kit'
import { useCallback, useEffect, useId, useState } from 'react'

import { loadList, type Refusal } from '@/app/load'
import { useSession } from '@/app/session-store'
import { useCurrentWorkspace } from '@/app/workspace-store'
import { HOST_NAME, MAX_PROFILES } from '@/features/hosts/enroll-store'
import { DEFAULT_EXPIRY_DAYS, decodeJoinClaims, epochToRfc3339, EXPIRY_DAYS } from '@/features/hosts/join'
import { parseProfiles } from '@/features/hosts/store'
import { useGovernance } from '@/features/teams/store'
import type { Event, JoinTokenInfo } from '@/protocol/wire'

import { canSeeSettings } from './tabs'

const SECTION = 'flex min-w-0 flex-col gap-3'

function days(n: number): string {
  return n === 1 ? '1 day' : `${n} days`
}

function ExpirySelect({ id, value, onChange }: { id: string; value: number; onChange: (n: number) => void }) {
  return (
    <Select
      id={id}
      value={String(value)}
      onChange={(e) => {
        onChange(Number(e.target.value))
      }}
    >
      {EXPIRY_DAYS.map((d) => (
        <option key={d} value={d}>
          {days(d)}
        </option>
      ))}
    </Select>
  )
}

/** The one copy of a token just issued or rotated, with what it tells a host. */
function Secret({ token, onDone }: { token: string; onDone: () => void }) {
  const claims = decodeJoinClaims(token)
  return (
    <div data-join-secret className="flex min-w-0 flex-col gap-3 border border-accent-neon/60 p-3">
      <p role="status" className="flex items-start gap-1.5 text-xs text-status-warn">
        <WarningIcon aria-hidden className="mt-0.5 shrink-0" />
        <span>Copy the token now. Keel keeps only its hash and will not show it again.</span>
      </p>
      <CopyBlock label="Join token" value={token} masked data="join-token" />
      {claims && (
        <DetailList label="What the token tells a host" columns={2}>
          <DetailRow term="Server" data="claim-server">
            <span className="font-mono break-all">{claims.server}</span>
          </DetailRow>
          <DetailRow term="Expires" data="claim-expires">
            <Time at={epochToRfc3339(claims.expiresAt)} />
          </DetailRow>
          <DetailRow term="Host CA" data="claim-host-ca">
            <span className="font-mono text-xs break-all">{claims.hostCa}</span>
          </DetailRow>
          <DetailRow term="Passkey origin" data="claim-origin">
            <span className="font-mono break-all">{claims.origin}</span>
          </DetailRow>
          <DetailRow term="Suggested inventory name" data="claim-inventory-name">
            <span className="font-mono break-all">{claims.inventoryName ?? '—'}</span>
          </DetailRow>
          <DetailRow term="Suggested profiles" data="claim-profiles">
            <span className="font-mono">{claims.profiles.join(', ') || '—'}</span>
          </DetailRow>
        </DetailList>
      )}
      <p className="max-w-prose text-xs text-muted-foreground">
        Put it where your base profile reads it, as a secret. A host that runs the profile asks to join and waits in
        Hosts → Join requests.
      </p>
      <div>
        <Button size="sm" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  )
}

function IssueForm({ workspace, onSecret }: { workspace: string; onSecret: (event: Event) => void }) {
  const propose = useGovernance((s) => s.propose)
  const [expiry, setExpiry] = useState<number>(DEFAULT_EXPIRY_DAYS)
  const [name, setName] = useState('')
  const [profiles, setProfiles] = useState('')
  const expiryId = useId()
  const nameId = useId()
  const profilesId = useId()
  const picked = parseProfiles(profiles)
  const nameOk = name === '' || HOST_NAME.test(name)
  const profilesOk = picked.length <= MAX_PROFILES
  return (
    <form
      aria-label="Issue join token"
      className="flex min-w-0 flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        if (!nameOk || !profilesOk) return
        propose(
          'Issue join token',
          {
            type: 'PreviewJoinTokenIssue',
            data: {
              workspace,
              expires_in_days: expiry,
              inventory_name: name === '' ? null : name,
              profiles: picked.length > 0 ? picked : null,
            },
          },
          onSecret,
        )
      }}
    >
      <div className="grid min-w-0 gap-3 sm:grid-cols-3">
        <Field label="Lasts" htmlFor={expiryId}>
          <ExpirySelect id={expiryId} value={expiry} onChange={setExpiry} />
        </Field>
        <Field
          label="Inventory name (optional)"
          htmlFor={nameId}
          hint={nameOk ? 'Suggested to every host; the host’s own name wins.' : 'Letters, digits, dots, dashes and underscores; starts with a letter or digit.'}
        >
          <input
            id={nameId}
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
          label="Profiles (optional)"
          htmlFor={profilesId}
          hint={profilesOk ? 'Prefilled when accepting, e.g. base, web.' : `At most ${MAX_PROFILES}.`}
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
      <div>
        <Button type="submit" size="sm" disabled={!nameOk || !profilesOk}>
          <PlusIcon aria-hidden />
          Issue join token
        </Button>
      </div>
    </form>
  )
}

function Active({ info, onSecret, onChanged }: { info: JoinTokenInfo; onSecret: (event: Event) => void; onChanged: (event: Event) => void }) {
  const propose = useGovernance((s) => s.propose)
  const [expiry, setExpiry] = useState<number>(DEFAULT_EXPIRY_DAYS)
  const expiryId = useId()
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <DetailList label="Join token" columns={2}>
        <DetailRow term="State" data="join-token-state">
          <span className={`${STATUS} text-status-ok`}>active</span>
        </DetailRow>
        <DetailRow term="Expires" data="join-token-expires">
          {info.expires_at ? <Time at={info.expires_at} /> : '—'}
        </DetailRow>
        <DetailRow term="Issued" data="join-token-created">
          {info.created_at ? <Time at={info.created_at} /> : '—'}
          {info.created_by && <span className="text-muted-foreground"> by {info.created_by}</span>}
        </DetailRow>
        <DetailRow term="Token id" data="join-token-jti">
          <span className="font-mono break-all text-muted-foreground">{info.jti ?? '—'}</span>
        </DetailRow>
        <DetailRow term="Suggested inventory name" data="join-token-inventory-name">
          <span className="font-mono break-all">{info.inventory_name ?? '—'}</span>
        </DetailRow>
        <DetailRow term="Suggested profiles" data="join-token-profiles">
          <span className="font-mono">{info.profiles.join(', ') || '—'}</span>
        </DetailRow>
      </DetailList>
      <div className="flex min-w-0 flex-wrap items-end gap-2">
        <Field label="New token lasts" htmlFor={expiryId}>
          <ExpirySelect id={expiryId} value={expiry} onChange={setExpiry} />
        </Field>
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            propose(
              'Rotate join token',
              { type: 'PreviewJoinTokenRotate', data: { workspace: info.workspace, expires_in_days: expiry } },
              onSecret,
            )
          }}
        >
          <ArrowsClockwiseIcon aria-hidden />
          Rotate
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="text-status-err"
          onClick={() => {
            propose('Disable join token', { type: 'PreviewJoinTokenDisable', data: { workspace: info.workspace } }, onChanged)
          }}
        >
          <ProhibitIcon aria-hidden />
          Disable
        </Button>
      </div>
      <p className="max-w-prose text-xs text-muted-foreground">
        Rotating or disabling rejects the requests still waiting on this token. Hosts that already joined keep working.
      </p>
    </div>
  )
}

/** Owners and admins: a new signing key retires every workspace's join token. */
function SigningKey({ onRotated }: { onRotated: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const [rotated, setRotated] = useState<{ kid: string; retired: number } | null>(null)
  return (
    <section aria-labelledby="join-signing-key-h" className={SECTION}>
      <h2 id="join-signing-key-h" className={H2}>
        Join signing key
      </h2>
      <p className="max-w-prose text-xs text-muted-foreground">
        Keel signs join tokens with its own key. Rotate it if a token or the key may have leaked: every workspace&apos;s
        join token stops working, and their admins issue new ones.
      </p>
      {rotated && (
        <p role="status" data-join-signing-key className="text-xs">
          New key <span className="font-mono">{rotated.kid}</span>. {rotated.retired === 1 ? '1 join token' : `${rotated.retired} join tokens`} retired.
        </p>
      )}
      <div>
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            propose('Rotate join signing key', { type: 'PreviewJoinSigningKeyRotate' }, (event) => {
              if (event.type === 'JoinSigningKey') {
                setRotated({ kid: event.data.kid, retired: event.data.retired_tokens })
                onRotated()
              }
            })
          }}
        >
          <KeyIcon aria-hidden />
          Rotate signing key
        </Button>
      </div>
    </section>
  )
}

/** One workspace's join token; keyed by workspace, so switching starts afresh. */
function JoinTokenPane({ id }: { id: string }) {
  const [info, setInfo] = useState<JoinTokenInfo | null>(null)
  const [failed, setFailed] = useState<Refusal | null>(null)
  const [secret, setSecret] = useState<string | null>(null)

  const load = useCallback(() => {
    loadList(
      { type: 'GetJoinToken', data: { workspace: id } },
      (event) => (event.type === 'JoinToken' && event.data.workspace === id ? event.data : undefined),
      {
        loaded: (value) => {
          setInfo(value)
          setFailed(null)
        },
        failed: setFailed,
      },
    )
  }, [id])

  useEffect(load, [load])

  // A confirm answers with the token once (issue, rotate) or with the new state (disable).
  const onSecret = (event: Event) => {
    if (event.type === 'JoinTokenSecret') {
      setSecret(event.data.token)
      setInfo(event.data.info)
    }
    if (event.type === 'Confirmed') load()
  }
  const onChanged = (event: Event) => {
    if (event.type === 'JoinToken') setInfo(event.data)
    if (event.type === 'Confirmed') load()
  }

  return (
    <>
        {secret !== null && (
          <Secret
            token={secret}
            onDone={() => {
              setSecret(null)
            }}
          />
        )}
          <LoadState
            loading={info === null && !failed}
            failed={failed}
            what="the join token"
            onRetry={() => {
              setFailed(null)
              load()
            }}
            label="Loading the join token…"
          >
            {info?.active ? (
              <Active info={info} onSecret={onSecret} onChanged={onChanged} />
            ) : (
              secret === null && (
                <div className="flex min-w-0 flex-col gap-3">
                  <p className="text-xs text-muted-foreground" data-join-token-state="none">
                    No join token. Hosts can only enrol one at a time, with an enrolment token.
                  </p>
                  <IssueForm workspace={id} onSecret={onSecret} />
                </div>
              )
            )}
          </LoadState>
    </>
  )
}

export default function HostsTab() {
  const workspace = useCurrentWorkspace()
  const role = useSession((s) => s.me?.instance_role)
  // A new signing key retires the token: remount the pane so it reads the token again.
  const [round, setRound] = useState(0)
  return (
    <div className="flex min-w-0 flex-col gap-6">
      <section aria-labelledby="join-token-h" data-pane="join-token" className={SECTION}>
        <h2 id="join-token-h" className={H2}>
          Join token{workspace ? <span className="font-normal text-muted-foreground"> · {workspace.name}</span> : null}
        </h2>
        <SectionIntro guide={{ guide: 'hosts', anchor: 'join-with-a-join-token' }}>
          One token for every host this workspace builds in bulk. A host that has it asks to join, and waits until an
          operator accepts it in Hosts → Join requests.
        </SectionIntro>
        {workspace ? (
          <JoinTokenPane key={`${workspace.id}:${round}`} id={workspace.id} />
        ) : (
          <p className="text-xs text-muted-foreground">Pick a workspace first.</p>
        )}
      </section>
      {canSeeSettings(role) && (
        <SigningKey
          onRotated={() => {
            setRound((n) => n + 1)
          }}
        />
      )}
    </div>
  )
}

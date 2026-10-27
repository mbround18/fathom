/**
 * Users (spec 055 US6 T038): everyone on the instance with their role and teams, and the signed-in
 * person's API tokens.
 *
 * Changing someone's role or removing them goes through the preview dialog and needs a passkey;
 * the server refuses demoting or removing the last owner, and its refusal is shown as sent. A
 * person who may not list users sees that refusal in place of the list. A new token's secret is
 * shown once, in the dialog.
 */
import { PlusIcon, ProhibitIcon, TrashIcon } from '@phosphor-icons/react'
import { Badge, Button, Input } from '@companion/ui'
import { useEffect, useId, useState } from 'react'

import { ChangeDialog } from '@/features/teams/ChangeDialog'
import { INSTANCE_ROLE_LABEL, INSTANCE_ROLES } from '@/features/teams/labels'
import { Field, InlineForm, ListState, NativeSelect, Pane } from '@/features/teams/parts'
import { useGovernance } from '@/features/teams/store'
import type { ApiTokenInfo, InstanceRole, UserInfo } from '@/protocol/wire'

const ROW = 'flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-t border-border px-2 py-1 text-xs'

/** Day precision is enough for a list; the full timestamp is in the title. */
function day(rfc3339: string): string {
  return rfc3339.slice(0, 10)
}

function UserRow({ user }: { user: UserInfo }) {
  const propose = useGovernance((s) => s.propose)
  const roleId = useId()
  return (
    <li data-user={user.display_name} className={ROW}>
      <span className="font-medium">{user.display_name}</span>
      {user.email && <span className="min-w-0 break-all text-muted-foreground">{user.email}</span>}
      {user.disabled && <Badge variant="destructive">removed</Badge>}
      {user.teams.length > 0 && (
        <span data-teams className="min-w-0 break-words text-muted-foreground">
          {user.teams.join(', ')}
        </span>
      )}
      <span className="flex-1" />
      {user.disabled ? (
        <Badge variant="outline">{INSTANCE_ROLE_LABEL[user.instance_role]}</Badge>
      ) : (
        <>
          <label htmlFor={roleId} className="sr-only">
            Role for {user.display_name}
          </label>
          <NativeSelect
            id={roleId}
            value={user.instance_role}
            onChange={(e) =>
              propose('Change role', {
                type: 'PreviewUserRole',
                data: { user: user.id, role: e.target.value as InstanceRole },
              })
            }
          >
            {INSTANCE_ROLES.map((r) => (
              <option key={r} value={r}>
                {INSTANCE_ROLE_LABEL[r]}
              </option>
            ))}
          </NativeSelect>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Remove ${user.display_name}`}
            onClick={() => propose('Remove user', { type: 'PreviewUserRemove', data: { user: user.id } })}
          >
            <TrashIcon />
          </Button>
        </>
      )}
    </li>
  )
}

function Users() {
  const users = useGovernance((s) => s.users)
  const refusal = useGovernance((s) => s.refused.users)
  return (
    <Pane id="users-list" title="People" count={users?.length}>
      <ListState items={users} refusal={refusal} empty="No users yet." />
      {users && users.length > 0 && (
        <ul aria-label="People" className="min-w-0">
          {users.map((u) => (
            <UserRow key={u.id} user={u} />
          ))}
        </ul>
      )}
    </Pane>
  )
}

/** Splits typed actions on commas and whitespace, dropping repeats. */
function actionList(text: string): string[] {
  return [...new Set(text.split(/[\s,]+/).filter(Boolean))]
}

function NewToken({ onDone }: { onDone: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const workspaces = useGovernance((s) => s.workspaces)
  const [name, setName] = useState('')
  const [workspace, setWorkspace] = useState('')
  const [actions, setActions] = useState('')
  const [days, setDays] = useState('30')
  const nameId = useId()
  const wsId = useId()
  const actionsId = useId()
  const daysId = useId()
  const n = Number(days)
  const ready = name.trim() !== '' && workspace !== '' && actionList(actions).length > 0 && Number.isInteger(n) && n >= 1 && n <= 366
  return (
    <InlineForm
      label="New API token"
      onSubmit={() => {
        if (!ready) return
        propose('New API token', {
          type: 'PreviewApiTokenCreate',
          data: { name: name.trim(), workspace, actions: actionList(actions), expires_in_days: n },
        })
        onDone()
      }}
    >
      <Field label="Token name" htmlFor={nameId}>
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" className="h-7 text-xs" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Workspace" htmlFor={wsId}>
        <NativeSelect id={wsId} value={workspace} onChange={(e) => setWorkspace(e.target.value)}>
          <option value="">Choose a workspace…</option>
          {(workspaces ?? [])
            .filter((w) => !w.archived)
            .map((w) => (
              <option key={w.id} value={w.id}>
                {w.slug}
              </option>
            ))}
        </NativeSelect>
      </Field>
      <Field label="Actions" htmlFor={actionsId}>
        <Input
          id={actionsId}
          value={actions}
          placeholder="list_workspaces, list_teams"
          autoComplete="off"
          className="h-7 font-mono text-xs"
          onChange={(e) => setActions(e.target.value)}
        />
      </Field>
      <Field label="Expires in days" htmlFor={daysId}>
        <Input id={daysId} type="number" min={1} max={366} value={days} className="h-7 text-xs" onChange={(e) => setDays(e.target.value)} />
      </Field>
      <Button type="submit" size="sm" disabled={!ready}>
        Preview
      </Button>
    </InlineForm>
  )
}

function TokenRow({ token }: { token: ApiTokenInfo }) {
  const propose = useGovernance((s) => s.propose)
  return (
    <li data-token={token.name} className={ROW}>
      <span className="font-medium">{token.name}</span>
      <span className="font-mono text-muted-foreground">{token.workspace_slug}</span>
      {token.revoked && <Badge variant="destructive">revoked</Badge>}
      <span className="min-w-0 break-all font-mono text-muted-foreground">{token.actions.join(', ')}</span>
      <span title={token.expires_at} className="text-muted-foreground tabular-nums">
        expires {day(token.expires_at)}
      </span>
      <span title={token.last_used_at ?? undefined} className="text-muted-foreground tabular-nums">
        {token.last_used_at ? `used ${day(token.last_used_at)}` : 'never used'}
      </span>
      <span className="flex-1" />
      {!token.revoked && (
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={`Revoke ${token.name}`}
          onClick={() => propose('Revoke API token', { type: 'PreviewApiTokenRevoke', data: { token: token.id } })}
        >
          <ProhibitIcon />
        </Button>
      )}
    </li>
  )
}

function Tokens() {
  const tokens = useGovernance((s) => s.tokens)
  const refusal = useGovernance((s) => s.refused.tokens)
  const [creating, setCreating] = useState(false)
  return (
    <Pane
      id="tokens"
      title="Your API tokens"
      count={tokens?.length}
      actions={
        <Button variant="outline" size="xs" aria-expanded={creating} onClick={() => setCreating((c) => !c)}>
          <PlusIcon aria-hidden />
          New token
        </Button>
      }
    >
      {creating && <NewToken onDone={() => setCreating(false)} />}
      <ListState items={tokens} refusal={refusal} empty="No API tokens." />
      {tokens && tokens.length > 0 && (
        <ul aria-label="API tokens" className="min-w-0">
          {tokens.map((t) => (
            <TokenRow key={t.id} token={t} />
          ))}
        </ul>
      )}
    </Pane>
  )
}

export default function UsersPage() {
  const load = useGovernance((s) => s.load)
  useEffect(() => {
    load('users')
    load('tokens')
    load('workspaces')
  }, [load])
  return (
    <section data-page="users" aria-labelledby="users-title" className="flex h-full min-w-0 flex-col overflow-y-auto">
      <div className="flex min-w-0 shrink-0 items-center gap-2 border-b border-border px-2 py-1.5">
        <h1 id="users-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Users
        </h1>
        <p className="min-w-0 text-xs text-muted-foreground">Who can sign in, their role, and API tokens.</p>
      </div>
      <Users />
      <Tokens />
      <ChangeDialog />
    </section>
  )
}

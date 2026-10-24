/**
 * `/admin/users` (US5, T074): who may do what. Admins admit people waiting for access, set roles
 * and stacks, pre-add an id that has never signed in, and remove people. Owners are locked: the
 * server refuses any change to them, so the view offers none. A stack assigned but not on disk is
 * kept and marked missing.
 */
import { LockSimpleIcon, PlusIcon, WarningIcon } from '@phosphor-icons/react'
import { Button } from '@companion/ui'
import { useEffect, useId, useMemo, useState, type SyntheticEvent, type ReactNode } from 'react'

import { DENY_TEXT } from '@/features/stacks/argv'
import { formatTime } from '@/features/stacks/model'
import { useStacks } from '@/features/stacks/store'
import type { Role, User } from '@/protocol/generated'

import { displayName, isUserId, missingStacks, sortUsers, stackChoices } from './model'
import { useUsers } from './store'

const NO_STACKS: readonly string[] = []

export default function UsersView() {
  const list = useUsers((s) => s.list)
  const denied = useUsers((s) => s.denied)
  const error = useUsers((s) => s.error)
  const clearError = useUsers((s) => s.clearError)
  const load = useUsers((s) => s.load)
  const loadStacks = useStacks((s) => s.load)
  const stackList = useStacks((s) => s.list)

  useEffect(() => {
    load()
    loadStacks()
  }, [load, loadStacks])

  const known = useMemo(() => stackList?.stacks.map((s) => s.name) ?? NO_STACKS, [stackList])
  const users = useMemo(() => sortUsers(list?.users ?? []), [list])
  const waiting = useMemo(() => sortUsers(list?.pending ?? []), [list])
  const allIds = useMemo(() => new Set([...users, ...waiting].map((u) => u.id)), [users, waiting])

  return (
    <section data-page="users" aria-labelledby="users-title" className="flex min-w-0 flex-col">
      <div className="flex min-w-0 flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
        <h1 id="users-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Users
        </h1>
        {list && (
          <span data-testid="user-count" className="text-xs text-muted-foreground tabular-nums">
            {users.length} · {waiting.length} waiting
          </span>
        )}
      </div>

      {error !== null && (
        <div role="alert" data-banner="denied" className="flex items-start gap-1.5 border-b border-status-warn/50 px-2 py-1.5 text-xs text-status-warn">
          <WarningIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          <p className="min-w-0 flex-1">{DENY_TEXT[error]}</p>
          <Button variant="ghost" size="xs" onClick={clearError}>
            Dismiss
          </Button>
        </div>
      )}

      {denied ? (
        <p role="status" className="p-3 text-xs text-muted-foreground">
          You can&apos;t see the users list.
        </p>
      ) : !list ? (
        <p role="status" data-pane-state="loading" className="p-3 text-xs text-muted-foreground">
          Loading users…
        </p>
      ) : (
        <div className="flex flex-col gap-3 p-2">
          <AddById existing={allIds} />
          <Group id="group-pending" label="Waiting for access" count={waiting.length}>
            {waiting.length === 0 ? (
              <Empty>No one is waiting.</Empty>
            ) : (
              waiting.map((u) => <UserRow key={u.id} user={u} known={known} pending />)
            )}
          </Group>
          <Group id="group-users" label="People with access" count={users.length}>
            {users.length === 0 ? <Empty>No users yet.</Empty> : users.map((u) => <UserRow key={u.id} user={u} known={known} />)}
          </Group>
        </div>
      )}
    </section>
  )
}

function Group({ id, label, count, children }: { id: string; label: string; count: number; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="flex min-w-0 flex-col gap-1">
      <h2 id={id} className="text-xs font-semibold tracking-wider text-muted-foreground uppercase">
        {label} <span className="tabular-nums">({count})</span>
      </h2>
      <ul className="flex min-w-0 flex-col gap-1">{children}</ul>
    </section>
  )
}

function Empty({ children }: { children: string }) {
  return <li className="text-xs text-muted-foreground">{children}</li>
}

/** Pre-add someone by provider id, before they ever sign in (FR-030). */
function AddById({ existing }: { existing: ReadonlySet<string> }) {
  const put = useUsers((s) => s.put)
  const [id, setId] = useState('')
  const [role, setRole] = useState<Role>('user')
  const [problem, setProblem] = useState<string | null>(null)
  const hint = useId()

  const submit = (e: SyntheticEvent): void => {
    e.preventDefault()
    const v = id.trim()
    if (!isUserId(v)) {
      setProblem('Use a provider id: discord:<id>, github:<numeric id> or oidc:<sub>.')
      return
    }
    if (existing.has(v)) {
      setProblem('That id is already listed.')
      return
    }
    setProblem(null)
    put(v, role, [])
    setId('')
  }

  return (
    <form onSubmit={submit} aria-label="Add a person by id" className="flex min-w-0 flex-wrap items-center gap-1 border border-border p-1.5">
      <input
        aria-label="Provider id"
        aria-describedby={hint}
        aria-invalid={problem !== null || undefined}
        placeholder="discord:… or github:…"
        value={id}
        onChange={(e) => setId(e.target.value)}
        className="h-7 min-w-0 flex-1 basis-48 border border-input bg-background px-1.5 font-mono text-xs outline-none placeholder:text-muted-foreground focus:border-ring aria-invalid:border-status-err"
      />
      <RoleSelect value={role} onChange={setRole} label="Role for the new id" />
      <Button type="submit" variant="outline" size="sm">
        <PlusIcon aria-hidden />
        Add
      </Button>
      <p id={hint} className={`basis-full text-xs ${problem ? 'text-status-err' : 'text-muted-foreground'}`}>
        {problem ?? 'They get access the first time they sign in with that account.'}
      </p>
    </form>
  )
}

function RoleSelect({ value, onChange, label }: { value: Role; onChange: (r: Role) => void; label: string }) {
  return (
    <select
      aria-label={label}
      value={value}
      onChange={(e) => onChange(e.target.value as Role)}
      className="h-7 shrink-0 border border-input bg-background px-1 text-xs focus-visible:border-ring"
    >
      <option value="user">User</option>
      <option value="admin">Admin</option>
    </select>
  )
}

function UserRow({ user, known, pending = false }: { user: User; known: readonly string[]; pending?: boolean }) {
  const saving = useUsers((s) => s.saving.includes(user.id))
  const remove = useUsers((s) => s.remove)
  const [editing, setEditing] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const knownSet = useMemo(() => new Set(known), [known])
  const missing = missingStacks(user, knownSet)
  const name = displayName(user)

  return (
    <li
      data-user={user.id}
      data-role={user.role}
      data-owner={user.owner || undefined}
      aria-busy={saving || undefined}
      className="flex min-w-0 flex-col gap-1 border border-border p-1.5 text-xs"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
        <span className="min-w-0 truncate font-semibold">{name}</span>
        <span className="min-w-0 truncate font-mono text-muted-foreground">{user.id}</span>
        {user.owner ? (
          <span data-badge="owner" className="inline-flex items-center gap-0.5 border border-primary/60 px-1 text-primary">
            <LockSimpleIcon aria-hidden className="size-3" />
            Owner
          </span>
        ) : (
          !pending && <span data-badge="role" className="border border-border px-1 uppercase">{user.role}</span>
        )}
        <span className="flex-1" />
        <span className="text-muted-foreground">
          {user.first_seen === null ? 'Never signed in' : `Last seen ${formatTime(user.last_seen)}`}
        </span>
      </div>

      {!pending && user.role === 'user' && (
        <ul aria-label={`Stacks for ${name}`} className="flex min-w-0 flex-wrap gap-1">
          {user.stacks.length === 0 && <li className="text-muted-foreground">No stacks: no access.</li>}
          {user.stacks.map((s) => (
            <li
              key={s}
              data-stack={s}
              data-missing={missing.includes(s) || undefined}
              className={`border px-1 ${missing.includes(s) ? 'border-status-warn/60 text-status-warn' : 'border-border'}`}
            >
              {s}
              {missing.includes(s) && <span className="ml-1">(missing)</span>}
            </li>
          ))}
        </ul>
      )}
      {!pending && user.role === 'admin' && !user.owner && <p className="text-muted-foreground">Every stack.</p>}

      {user.owner ? (
        <p className="text-muted-foreground">Set by the server&apos;s owner setting; it cannot be changed or removed here.</p>
      ) : editing ? (
        <GrantEditor user={user} known={known} pending={pending} onDone={() => setEditing(false)} />
      ) : confirming ? (
        <div role="group" aria-label={`Remove ${name}?`} className="flex flex-wrap items-center gap-1">
          <span className="text-status-err">Remove {name}? Their sessions end now.</span>
          <Button variant="destructive" size="xs" onClick={() => remove(user.id)} disabled={saving}>
            Remove
          </Button>
          <Button variant="ghost" size="xs" onClick={() => setConfirming(false)}>
            Keep
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap gap-1">
          <Button variant="outline" size="xs" onClick={() => setEditing(true)} aria-label={`${pending ? 'Admit' : 'Edit'} ${name}`}>
            {pending ? 'Admit' : 'Edit'}
          </Button>
          {!pending && (
            <Button variant="ghost" size="xs" onClick={() => setConfirming(true)} aria-label={`Remove ${name}`}>
              Remove
            </Button>
          )}
        </div>
      )}
    </li>
  )
}

function GrantEditor({ user, known, pending, onDone }: { user: User; known: readonly string[]; pending: boolean; onDone: () => void }) {
  const put = useUsers((s) => s.put)
  const [role, setRole] = useState<Role>(user.role === 'admin' ? 'admin' : 'user')
  const [stacks, setStacks] = useState<string[]>(user.stacks)
  const choices = useMemo(() => stackChoices(known, user.stacks), [known, user.stacks])
  const knownSet = useMemo(() => new Set(known), [known])
  const name = displayName(user)

  const toggle = (s: string): void => setStacks((cur) => (cur.includes(s) ? cur.filter((x) => x !== s) : [...cur, s]))

  const save = (e: SyntheticEvent): void => {
    e.preventDefault()
    // Admins see every stack; the list is kept as given in case they are made a user again.
    put(user.id, role, choices.filter((s) => stacks.includes(s)))
    onDone()
  }

  return (
    <form onSubmit={save} aria-label={`Access for ${name}`} className="flex min-w-0 flex-col gap-1 border-t border-border pt-1">
      <div className="flex items-center gap-1">
        <RoleSelect value={role} onChange={setRole} label={`Role for ${name}`} />
      </div>
      {role === 'user' && (
        <fieldset className="flex min-w-0 flex-wrap gap-x-3 gap-y-1">
          <legend className="mb-0.5 text-muted-foreground">Stacks</legend>
          {choices.length === 0 && <span className="text-muted-foreground">No stacks on disk.</span>}
          {choices.map((s) => (
            <label key={s} className="inline-flex items-center gap-1">
              <input type="checkbox" checked={stacks.includes(s)} onChange={() => toggle(s)} className="accent-primary" />
              <span>{s}</span>
              {!knownSet.has(s) && <span className="text-status-warn">(missing)</span>}
            </label>
          ))}
        </fieldset>
      )}
      {role === 'user' && stacks.length === 0 && (
        <p className="text-status-warn">With no stacks, {name} has no access{pending ? '' : ' and is signed out'}.</p>
      )}
      <div className="flex gap-1">
        <Button type="submit" size="xs">
          Save
        </Button>
        <Button type="button" variant="ghost" size="xs" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  )
}

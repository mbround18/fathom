/**
 * Teams (spec 055 US6 T038): workspaces, the teams with their members, and each team's grants.
 *
 * Dense lists, mobile first: one row per object, actions at the row's end, forms inline under the
 * pane header. Every change goes through the preview dialog (./ChangeDialog.tsx); the page never
 * hides an action it guesses the person may not take: the server decides, and its refusal is
 * shown as sent.
 */
import { ArchiveIcon, CaretDownIcon, CaretRightIcon, FolderPlusIcon, PlusIcon, TrashIcon } from '@phosphor-icons/react'
import { Badge, Button, Input } from '@companion/ui'
import { useEffect, useId, useState } from 'react'

import type { GrantInfo, TeamInfo, WorkspaceInfo } from '@/protocol/wire'

import { ChangeDialog } from './ChangeDialog'
import { GrantEditor } from './GrantEditor'
import { ROLE_LABEL } from './labels'
import { Field, InlineForm, ListState, NativeSelect, Pane } from './parts'
import { useGovernance } from './store'

const ROW = 'flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-t border-border px-2 py-1 text-xs'

function NewWorkspace({ onDone }: { onDone: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const [slug, setSlug] = useState('')
  const [name, setName] = useState('')
  const slugId = useId()
  const nameId = useId()
  return (
    <InlineForm
      label="New workspace"
      onSubmit={() => {
        if (!slug.trim() || !name.trim()) return
        propose('New workspace', { type: 'PreviewWorkspaceCreate', data: { slug: slug.trim(), name: name.trim() } })
        onDone()
      }}
    >
      <Field label="Slug" htmlFor={slugId}>
        <Input id={slugId} value={slug} maxLength={63} pattern="[a-z0-9][a-z0-9\-]{0,62}" title="Lowercase letters, digits and dashes" autoComplete="off" className="h-7 text-xs" onChange={(e) => setSlug(e.target.value)} />
      </Field>
      <Field label="Name" htmlFor={nameId}>
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" className="h-7 text-xs" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Button type="submit" size="sm" disabled={!slug.trim() || !name.trim()}>
        Preview
      </Button>
    </InlineForm>
  )
}

function NewProject({ workspace, onDone }: { workspace: WorkspaceInfo; onDone: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const [slug, setSlug] = useState('')
  const [name, setName] = useState('')
  const slugId = useId()
  const nameId = useId()
  return (
    <InlineForm
      label={`New project in ${workspace.slug}`}
      onSubmit={() => {
        if (!slug.trim() || !name.trim()) return
        propose('New project', {
          type: 'PreviewProjectCreate',
          data: { workspace: workspace.id, slug: slug.trim(), name: name.trim() },
        })
        onDone()
      }}
    >
      <Field label="Project slug" htmlFor={slugId}>
        <Input id={slugId} value={slug} maxLength={63} pattern="[a-z0-9][a-z0-9\-]{0,62}" title="Lowercase letters, digits and dashes" autoComplete="off" className="h-7 text-xs" onChange={(e) => setSlug(e.target.value)} />
      </Field>
      <Field label="Project name" htmlFor={nameId}>
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" className="h-7 text-xs" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Button type="submit" size="sm" disabled={!slug.trim() || !name.trim()}>
        Preview
      </Button>
    </InlineForm>
  )
}

function WorkspaceRow({ ws }: { ws: WorkspaceInfo }) {
  const propose = useGovernance((s) => s.propose)
  const [adding, setAdding] = useState(false)
  return (
    <li data-workspace={ws.slug} className="flex min-w-0 flex-col">
      <div className={ROW}>
        <span className="font-mono">{ws.slug}</span>
        <span className="min-w-0 break-words text-muted-foreground">{ws.name}</span>
        {ws.archived && <Badge variant="outline">archived</Badge>}
        {ws.projects.length > 0 && (
          <span data-projects className="min-w-0 break-words text-muted-foreground">
            {ws.projects.map((p) => p.slug).join(', ')}
          </span>
        )}
        <span className="flex-1" />
        {!ws.archived && (
          <>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={`New project in ${ws.slug}`}
              aria-expanded={adding}
              onClick={() => setAdding((a) => !a)}
            >
              <FolderPlusIcon />
            </Button>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={`Archive ${ws.slug}`}
              onClick={() => propose('Archive workspace', { type: 'PreviewWorkspaceArchive', data: { workspace: ws.id } })}
            >
              <ArchiveIcon />
            </Button>
          </>
        )}
      </div>
      {adding && <NewProject workspace={ws} onDone={() => setAdding(false)} />}
    </li>
  )
}

function Workspaces() {
  const workspaces = useGovernance((s) => s.workspaces)
  const refusal = useGovernance((s) => s.refused.workspaces)
  const [creating, setCreating] = useState(false)
  return (
    <Pane
      id="workspaces"
      title="Workspaces"
      count={workspaces?.length}
      actions={
        <Button variant="outline" size="xs" aria-expanded={creating} onClick={() => setCreating((c) => !c)}>
          <PlusIcon aria-hidden />
          New workspace
        </Button>
      }
    >
      {creating && <NewWorkspace onDone={() => setCreating(false)} />}
      <ListState items={workspaces} refusal={refusal} empty="No workspaces yet." />
      {workspaces && workspaces.length > 0 && (
        <ul aria-label="Workspaces" className="min-w-0">
          {workspaces.map((ws) => (
            <WorkspaceRow key={ws.id} ws={ws} />
          ))}
        </ul>
      )}
    </Pane>
  )
}

function NewTeam({ onDone }: { onDone: () => void }) {
  const propose = useGovernance((s) => s.propose)
  const [name, setName] = useState('')
  const nameId = useId()
  return (
    <InlineForm
      label="New team"
      onSubmit={() => {
        if (!name.trim()) return
        propose('New team', { type: 'PreviewTeamCreate', data: { name: name.trim() } })
        onDone()
      }}
    >
      <Field label="Team name" htmlFor={nameId}>
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" className="h-7 text-xs" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Button type="submit" size="sm" disabled={!name.trim()}>
        Preview
      </Button>
    </InlineForm>
  )
}

/** Adds a member: a user from the list when the person may list users, else a user id. */
function AddMember({ team }: { team: TeamInfo }) {
  const propose = useGovernance((s) => s.propose)
  const users = useGovernance((s) => s.users)
  const [user, setUser] = useState('')
  const userId = useId()
  const candidates = users?.filter((u) => !u.disabled && !team.members.some((m) => m.user === u.id)) ?? null
  return (
    <InlineForm
      label={`Add a member to ${team.name}`}
      onSubmit={() => {
        if (!user.trim()) return
        propose('Add member', { type: 'PreviewTeamMemberAdd', data: { team: team.id, user: user.trim() } })
        setUser('')
      }}
    >
      <Field label="User" htmlFor={userId}>
        {candidates ? (
          <NativeSelect id={userId} value={user} onChange={(e) => setUser(e.target.value)}>
            <option value="">Choose a user…</option>
            {candidates.map((u) => (
              <option key={u.id} value={u.id}>
                {u.display_name}
                {u.email ? ` (${u.email})` : ''}
              </option>
            ))}
          </NativeSelect>
        ) : (
          <Input id={userId} value={user} placeholder="User id" autoComplete="off" className="h-7 text-xs" onChange={(e) => setUser(e.target.value)} />
        )}
      </Field>
      <Button type="submit" size="sm" disabled={!user.trim()}>
        Add member
      </Button>
    </InlineForm>
  )
}

function grantLabel(g: GrantInfo): string {
  return `${ROLE_LABEL[g.role]} on ${g.workspace_slug}`
}

function TeamRow({ team, knownProfiles }: { team: TeamInfo; knownProfiles: readonly string[] }) {
  const propose = useGovernance((s) => s.propose)
  const [open, setOpen] = useState(false)
  const bodyId = useId()
  return (
    <li data-team={team.name} className="flex min-w-0 flex-col border-t border-border">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={() => setOpen((o) => !o)}
        className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 px-2 py-1.5 text-left text-xs hover:bg-muted/40"
      >
        {open ? <CaretDownIcon aria-hidden className="size-3" /> : <CaretRightIcon aria-hidden className="size-3" />}
        <span className="font-medium">{team.name}</span>
        <span className="text-muted-foreground tabular-nums">
          {team.members.length} {team.members.length === 1 ? 'member' : 'members'}
        </span>
        <span className="min-w-0 break-words text-muted-foreground">
          {team.grants.length === 0 ? 'no grants' : team.grants.map(grantLabel).join(', ')}
        </span>
      </button>
      {open && (
        <div id={bodyId} className="flex min-w-0 flex-col pb-1 pl-3">
          <h3 className="px-2 pt-1 text-xs text-muted-foreground">Members</h3>
          <ul aria-label={`Members of ${team.name}`} className="min-w-0">
            {team.members.map((m) => (
              <li key={m.user} data-member={m.display_name} className={ROW}>
                <span className="min-w-0 flex-1 break-words">{m.display_name}</span>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove ${m.display_name} from ${team.name}`}
                  onClick={() =>
                    propose('Remove member', { type: 'PreviewTeamMemberRemove', data: { team: team.id, user: m.user } })
                  }
                >
                  <TrashIcon />
                </Button>
              </li>
            ))}
          </ul>
          <AddMember team={team} />
          <h3 className="px-2 pt-2 text-xs text-muted-foreground">Grants</h3>
          <ul aria-label={`Grants of ${team.name}`} className="min-w-0">
            {team.grants.map((g) => (
              <li key={g.id} data-grant={`${g.workspace_slug}:${g.role}`} className={ROW}>
                <span className="font-mono">{g.workspace_slug}</span>
                <Badge variant="secondary">{ROLE_LABEL[g.role]}</Badge>
                <span className="min-w-0 flex-1 break-words text-muted-foreground">
                  {g.profiles.length === 0 ? 'whole workspace' : g.profiles.join(', ')}
                </span>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Remove grant ${grantLabel(g)}`}
                  onClick={() => propose('Remove grant', { type: 'PreviewGrantRemove', data: { grant: g.id } })}
                >
                  <TrashIcon />
                </Button>
              </li>
            ))}
          </ul>
          <GrantEditor team={team} knownProfiles={knownProfiles} />
        </div>
      )}
    </li>
  )
}

function Teams() {
  const teams = useGovernance((s) => s.teams)
  const refusal = useGovernance((s) => s.refused.teams)
  const [creating, setCreating] = useState(false)
  const knownProfiles = [...new Set((teams ?? []).flatMap((t) => t.grants.flatMap((g) => g.profiles)))].sort()
  return (
    <Pane
      id="teams-list"
      title="Teams"
      count={teams?.length}
      actions={
        <Button variant="outline" size="xs" aria-expanded={creating} onClick={() => setCreating((c) => !c)}>
          <PlusIcon aria-hidden />
          New team
        </Button>
      }
    >
      {creating && <NewTeam onDone={() => setCreating(false)} />}
      <ListState items={teams} refusal={refusal} empty="No teams yet." />
      {teams && teams.length > 0 && (
        <ul aria-label="Teams" className="min-w-0">
          {teams.map((t) => (
            <TeamRow key={t.id} team={t} knownProfiles={knownProfiles} />
          ))}
        </ul>
      )}
    </Pane>
  )
}

export default function TeamsPage() {
  const load = useGovernance((s) => s.load)
  useEffect(() => {
    load('workspaces')
    load('teams')
    // For the member picker; a refusal here only means the picker takes a user id instead.
    load('users')
  }, [load])
  return (
    <section data-page="teams" aria-labelledby="teams-title" className="flex h-full min-w-0 flex-col overflow-y-auto">
      <div className="flex min-w-0 shrink-0 items-center gap-2 border-b border-border px-2 py-1.5">
        <h1 id="teams-title" className="font-display text-sm font-semibold tracking-wider uppercase">
          Teams
        </h1>
        <p className="min-w-0 text-xs text-muted-foreground">Workspaces, their members and what they may run.</p>
      </div>
      <Workspaces />
      <Teams />
      <ChangeDialog />
    </section>
  )
}

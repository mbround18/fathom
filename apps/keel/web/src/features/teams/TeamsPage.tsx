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
import { CopyBlock, EmptyState, H3, Page, Select } from '@companion/ui/kit'
import { useEffect, useId, useState } from 'react'

import { useSession } from '@/app/session-store'
import { canSeeSettings } from '@/features/settings/tabs'
import { TermTip } from '@/help/TermTip'
import type { GrantInfo, TeamInfo, WorkspaceInfo } from '@/protocol/wire'
import { PageHeader } from '@/ui/PageHeader'

import { ChangeDialog } from './ChangeDialog'
import { GrantEditor } from './GrantEditor'
import { ROLE_LABEL } from './labels'

import { Field, InlineForm, ListState, Pane } from './parts'
import { useGovernance } from './store'

const ROW = 'flex min-h-10 min-w-0 flex-wrap items-center gap-x-3 gap-y-1 border-t border-border px-4 py-2 text-sm first:border-t-0'

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
        <Input id={slugId} value={slug} maxLength={63} pattern="[a-z0-9][a-z0-9\-]{0,62}" title="Lowercase letters, digits and dashes" autoComplete="off" onChange={(e) => setSlug(e.target.value)} />
      </Field>
      <Field label="Name" htmlFor={nameId}>
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Button type="submit" disabled={!slug.trim() || !name.trim()}>
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
        <Input id={slugId} value={slug} maxLength={63} pattern="[a-z0-9][a-z0-9\-]{0,62}" title="Lowercase letters, digits and dashes" autoComplete="off" onChange={(e) => setSlug(e.target.value)} />
      </Field>
      <Field label="Project name" htmlFor={nameId}>
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Button type="submit" disabled={!slug.trim() || !name.trim()}>
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
        <span className="flex-1" />
        {!ws.archived && (
          <>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={`New project in ${ws.slug}`}
              aria-expanded={adding}
              onClick={() => setAdding((a) => !a)}
            >
              <FolderPlusIcon />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={`Archive ${ws.slug}`}
              onClick={() => propose('Archive workspace', { type: 'PreviewWorkspaceArchive', data: { workspace: ws.id } })}
            >
              <ArchiveIcon />
            </Button>
          </>
        )}
      </div>
      {ws.projects.length > 0 && (
        <ul data-projects aria-label={`Projects in ${ws.slug}`} className="flex min-w-0 flex-col gap-2 px-4 pb-2">
          {ws.projects.map((p) => (
            <li key={p.id} data-project={p.slug} className="min-w-0">
              <CopyBlock label={`Project ${p.slug} id`} value={p.id} data="project-id" />
            </li>
          ))}
        </ul>
      )}
      {adding && <NewProject workspace={ws} onDone={() => setAdding(false)} />}
    </li>
  )
}

/** Owners and admins create workspaces and teams; the guided states offer them the first step. */
function useCanAdminister(): boolean {
  return canSeeSettings(useSession((s) => s.me?.instance_role))
}

function Workspaces() {
  const workspaces = useGovernance((s) => s.workspaces)
  const refusal = useGovernance((s) => s.refused.workspaces)
  const load = useGovernance((s) => s.load)
  const canAct = useCanAdminister()
  const [creating, setCreating] = useState(false)
  return (
    <Pane
      id="workspaces"
      title="Workspaces"
      tip={<TermTip term="workspace" />}
      count={workspaces?.length}
      actions={
        <Button variant="outline" size="sm" aria-expanded={creating} onClick={() => setCreating((c) => !c)}>
          <PlusIcon aria-hidden />
          New workspace
        </Button>
      }
    >
      {creating && <NewWorkspace onDone={() => setCreating(false)} />}
      {!creating && (
        <ListState
          items={workspaces}
          refusal={refusal}
          what="workspaces"
          onRetry={() => {
            load('workspaces')
          }}
          empty={
            <EmptyState
              page="workspaces"
              title={canAct ? 'Create a workspace' : 'No workspaces yet'}
              reason="Hosts, bindings and runs all live in a workspace."
              steps={['Create a workspace.', 'Grant a team access to it.', 'Enroll a host into it.']}
              action={
                canAct ? (
                  <Button
                    size="sm"
                    onClick={() => {
                      setCreating(true)
                    }}
                  >
                    <PlusIcon aria-hidden />
                    Create a workspace
                  </Button>
                ) : undefined
              }
              whoCan="An owner or admin"
              verb="create workspaces"
              guide={{ guide: 'getting-started', anchor: 'create-a-workspace' }}
            />
          }
        />
      )}
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
        <Input id={nameId} value={name} maxLength={64} autoComplete="off" onChange={(e) => setName(e.target.value)} />
      </Field>
      <Button type="submit" disabled={!name.trim()}>
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
          <Select id={userId} value={user} onChange={(e) => setUser(e.target.value)}>
            <option value="">Choose a user…</option>
            {candidates.map((u) => (
              <option key={u.id} value={u.id}>
                {u.display_name}
                {u.email ? ` (${u.email})` : ''}
              </option>
            ))}
          </Select>
        ) : (
          <Input id={userId} value={user} placeholder="User id" autoComplete="off" onChange={(e) => setUser(e.target.value)} />
        )}
      </Field>
      <Button type="submit" disabled={!user.trim()}>
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
    <li data-team={team.name} className="flex min-w-0 flex-col border-t border-border first:border-t-0">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={() => setOpen((o) => !o)}
        className="flex min-h-11 w-full min-w-0 flex-wrap items-center gap-x-3 gap-y-1 px-4 py-2 text-left text-sm hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:ring-1 focus-visible:ring-accent-neon focus-visible:outline-none focus-visible:ring-inset"
      >
        {open ? <CaretDownIcon aria-hidden className="size-3.5" /> : <CaretRightIcon aria-hidden className="size-3.5" />}
        <span className="font-semibold">{team.name}</span>
        <span className="text-muted-foreground tabular-nums">
          {team.members.length} {team.members.length === 1 ? 'member' : 'members'}
        </span>
        <span className="min-w-0 break-words text-muted-foreground">
          {team.grants.length === 0 ? 'no grants' : team.grants.map(grantLabel).join(', ')}
        </span>
      </button>
      {/* Members and grants side by side on a wide screen, one after the other on a phone: two
          plain sections under the team, not a third level of boxes. */}
      {open && (
        <div id={bodyId} className="grid min-w-0 border-t border-border lg:grid-cols-2 lg:divide-x lg:divide-border">
          <section className="flex min-w-0 flex-col pt-3">
            <h3 className={`px-4 pb-1 ${H3}`}>Members</h3>
            <ul aria-label={`Members of ${team.name}`} className="min-w-0">
              {team.members.map((m) => (
                <li key={m.user} data-member={m.display_name} className={ROW}>
                  <span className="min-w-0 flex-1 break-words">{m.display_name}</span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
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
          </section>
          <section className="flex min-w-0 flex-col border-t border-border pt-3 lg:border-t-0">
            <div className="flex min-w-0 items-center gap-1.5 px-4 pb-1">
              <h3 className={H3}>Grants</h3>
              <TermTip term="grant" />
            </div>
            <ul aria-label={`Grants of ${team.name}`} className="min-w-0">
              {team.grants.map((g) => (
                <li key={g.id} data-grant={`${g.workspace_slug}:${g.role}`} className={ROW}>
                  <span className="font-mono">{g.workspace_slug}</span>
                  <span data-grant-role>{ROLE_LABEL[g.role]}</span>
                  <span className="min-w-0 flex-1 break-words text-muted-foreground">
                    {g.profiles.length === 0 ? 'whole workspace' : g.profiles.join(', ')}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Remove grant ${grantLabel(g)}`}
                    onClick={() => propose('Remove grant', { type: 'PreviewGrantRemove', data: { grant: g.id } })}
                  >
                    <TrashIcon />
                  </Button>
                </li>
              ))}
            </ul>
            <GrantEditor team={team} knownProfiles={knownProfiles} />
          </section>
        </div>
      )}
    </li>
  )
}

function Teams() {
  const teams = useGovernance((s) => s.teams)
  const refusal = useGovernance((s) => s.refused.teams)
  const load = useGovernance((s) => s.load)
  const canAct = useCanAdminister()
  const [creating, setCreating] = useState(false)
  const knownProfiles = [...new Set((teams ?? []).flatMap((t) => t.grants.flatMap((g) => g.profiles)))].sort()
  return (
    <Pane
      id="teams-list"
      title="Teams"
      tip={<TermTip term="team" />}
      count={teams?.length}
      actions={
        <Button variant="outline" size="sm" aria-expanded={creating} onClick={() => setCreating((c) => !c)}>
          <PlusIcon aria-hidden />
          New team
        </Button>
      }
    >
      {creating && <NewTeam onDone={() => setCreating(false)} />}
      {!creating && (
        <ListState
          items={teams}
          refusal={refusal}
          what="teams"
          onRetry={() => {
            load('teams')
          }}
          empty={
            <EmptyState
              page="teams"
              title={canAct ? 'Create a team' : 'No teams yet'}
              reason="Teams carry workspace grants to their members."
              steps={['Create a team.', 'Add members.', 'Grant it a workspace.']}
              action={
                canAct ? (
                  <Button
                    size="sm"
                    onClick={() => {
                      setCreating(true)
                    }}
                  >
                    <PlusIcon aria-hidden />
                    Create a team
                  </Button>
                ) : undefined
              }
              whoCan="An owner or admin"
              verb="create teams"
              guide={{ guide: 'getting-started', anchor: 'give-people-access' }}
            />
          }
        />
      )}
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
    <Page name="teams" labelledBy="teams-title">
      <PageHeader
        id="teams-title"
        title="Teams"
        description="Workspaces, the teams in them, and what each team may run."
        guide={{ guide: 'getting-started', anchor: 'give-people-access' }}
      />
      <Workspaces />
      <Teams />
      <ChangeDialog />
    </Page>
  )
}

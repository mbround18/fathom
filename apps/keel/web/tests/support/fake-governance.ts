/**
 * Governance on the fake backend (spec 055 US6 T034): workspaces, teams, grants, users and API
 * tokens, with the server's preview → confirm flow.
 *
 * It installs itself with `backend.on(...)`, so fake-backend.ts stays untouched. Each `Preview*`
 * answers a `Preview` whose `affected` lists what apps/keel/server/src/governance.rs lists (a grant
 * reaches every member of the team), and `Confirm` applies it. `deny` scripts a refusal: the
 * preview answers `Denied { action: <the command's snake_case name>, reason }`, as the server
 * does for a preview-time refusal. IDs and timestamps are fixed, so stories stay deterministic.
 */
import type {
  ApiTokenInfo,
  Event,
  GrantInfo,
  TeamInfo,
  UserInfo,
  WorkspaceInfo,
} from '../../src/protocol/wire'
import type { CmdOf, CmdType, FakeBackend } from './fake-backend'

const NOW = '2026-10-27T12:00:00Z'
const EXPIRES = '2026-10-27T12:15:00Z'

const PREVIEWS = [
  'PreviewWorkspaceCreate',
  'PreviewWorkspaceUpdate',
  'PreviewWorkspaceArchive',
  'PreviewProjectCreate',
  'PreviewTeamCreate',
  'PreviewTeamMemberAdd',
  'PreviewTeamMemberRemove',
  'PreviewGrantAdd',
  'PreviewGrantRemove',
  'PreviewUserRole',
  'PreviewUserRemove',
  'PreviewApiTokenCreate',
  'PreviewApiTokenRevoke',
] as const satisfies readonly CmdType[]

type PreviewType = (typeof PREVIEWS)[number]

/** `PreviewGrantAdd` → `preview_grant_add`, as `Cmd::name` spells it. */
export function cmdName(type: string): string {
  return type.replace(/[A-Z]/g, (c, i: number) => (i === 0 ? '' : '_') + c.toLowerCase())
}

interface Draft {
  action: string
  summary: string
  affected: string[]
  step_up: boolean
  apply: () => Event[]
}

export const people = {
  ada: (): UserInfo => ({
    id: '01J0000000000000000000ADA',
    display_name: 'Ada Lovelace',
    email: 'ada@example.com',
    instance_role: 'owner',
    disabled: false,
    teams: [],
    created: NOW,
  }),
  grace: (): UserInfo => ({
    id: '01J000000000000000000GRACE',
    display_name: 'Grace Hopper',
    email: 'grace@example.com',
    instance_role: 'user',
    disabled: false,
    teams: [],
    created: NOW,
  }),
  linus: (): UserInfo => ({
    id: '01J000000000000000000LINUS',
    display_name: 'Linus Torvalds',
    email: 'linus@example.com',
    instance_role: 'user',
    disabled: false,
    teams: [],
    created: NOW,
  }),
}

export class FakeGovernance {
  workspaces: WorkspaceInfo[] = []
  teams: TeamInfo[] = []
  users: UserInfo[] = [people.ada(), people.grace(), people.linus()]
  tokens: ApiTokenInfo[] = []
  /** Scripted refusals: a `Preview*` or `List*` command answers `Denied` with this reason. */
  readonly deny = new Map<CmdType, string>()

  private readonly previews = new Map<string, Draft>()
  private seq = 0

  install(backend: FakeBackend): this {
    backend.on('ListWorkspaces', () => this.refused('ListWorkspaces') ?? [{ type: 'Workspaces', data: { items: this.workspaces } }])
    backend.on('ListTeams', () => this.refused('ListTeams') ?? [{ type: 'Teams', data: { items: this.teams } }])
    backend.on('ListUsers', () => this.refused('ListUsers') ?? [{ type: 'Users', data: { items: this.withTeams() } }])
    backend.on('ListApiTokens', () => this.refused('ListApiTokens') ?? [{ type: 'ApiTokens', data: { items: this.tokens } }])
    for (const type of PREVIEWS) {
      backend.on(type, (cmd) => this.refused(type) ?? this.preview(this.draft(cmd)))
    }
    backend.on('Confirm', (cmd) => {
      const draft = this.previews.get(cmd.data.preview_id)
      if (!draft) return [{ type: 'OpError', data: { code: 'preview_expired', message: 'That preview has expired.' } }]
      this.previews.delete(cmd.data.preview_id)
      return [{ type: 'Confirmed', data: { preview_id: cmd.data.preview_id } }, ...draft.apply()]
    })
    return this
  }

  /** Adds a workspace; returns it. */
  workspace(slug: string, name = slug): WorkspaceInfo {
    const ws: WorkspaceInfo = { id: this.id('ws'), slug, name, created: NOW, archived: false, projects: [] }
    this.workspaces.push(ws)
    return ws
  }

  /** Adds a team with these members; returns it. */
  team(name: string, members: readonly UserInfo[] = [], grants: readonly Omit<GrantInfo, 'id'>[] = []): TeamInfo {
    const team: TeamInfo = {
      id: this.id('team'),
      name,
      members: members.map((u) => ({ user: u.id, display_name: u.display_name })),
      grants: grants.map((g) => ({ ...g, id: this.id('grant') })),
    }
    this.teams.push(team)
    return team
  }

  private id(kind: string): string {
    return `${kind}-${String(++this.seq)}`
  }

  private refused(type: CmdType): Event[] | undefined {
    const reason = this.deny.get(type)
    return reason === undefined ? undefined : [{ type: 'Denied', data: { action: cmdName(type), reason } }]
  }

  private withTeams(): UserInfo[] {
    return this.users.map((u) => ({
      ...u,
      teams: this.teams.filter((t) => t.members.some((m) => m.user === u.id)).map((t) => t.name),
    }))
  }

  private preview(draft: Draft): Event[] {
    const preview_id = this.id('preview')
    this.previews.set(preview_id, draft)
    const { action, summary, affected, step_up } = draft
    return [{ type: 'Preview', data: { preview_id, action, summary, affected, step_up, expires_at: EXPIRES } }]
  }

  private ws(id: string): WorkspaceInfo {
    const ws = this.workspaces.find((w) => w.id === id)
    if (!ws) throw new Error(`no workspace ${id}`)
    return ws
  }

  private teamOf(id: string): TeamInfo {
    const t = this.teams.find((x) => x.id === id)
    if (!t) throw new Error(`no team ${id}`)
    return t
  }

  private user(id: string): UserInfo {
    const u = this.users.find((x) => x.id === id)
    if (!u) throw new Error(`no user ${id}`)
    return u
  }

  private draft(cmd: CmdOf<PreviewType>): Draft {
    switch (cmd.type) {
      case 'PreviewWorkspaceCreate': {
        const { slug, name } = cmd.data
        return {
          action: 'workspace_create',
          summary: `Create workspace ${slug} (${name})`,
          affected: [],
          step_up: false,
          apply: () => (this.workspace(slug, name), []),
        }
      }
      case 'PreviewWorkspaceUpdate': {
        const ws = this.ws(cmd.data.workspace)
        const { name } = cmd.data
        return { action: 'workspace_update', summary: `Rename ${ws.slug} to ${name}`, affected: [], step_up: false, apply: () => ((ws.name = name), []) }
      }
      case 'PreviewWorkspaceArchive': {
        const ws = this.ws(cmd.data.workspace)
        const affected = this.teams.filter((t) => t.grants.some((g) => g.workspace === ws.id)).map((t) => `team ${t.name}`)
        return { action: 'workspace_archive', summary: `Archive workspace ${ws.slug}`, affected, step_up: false, apply: () => ((ws.archived = true), []) }
      }
      case 'PreviewProjectCreate': {
        const ws = this.ws(cmd.data.workspace)
        const { slug, name } = cmd.data
        return {
          action: 'project_create',
          summary: `Create project ${slug} in ${ws.slug}`,
          affected: [],
          step_up: false,
          apply: () => (ws.projects.push({ id: this.id('project'), slug, name }), []),
        }
      }
      case 'PreviewTeamCreate': {
        const { name } = cmd.data
        return { action: 'team_create', summary: `Create team ${name}`, affected: [], step_up: false, apply: () => (this.team(name), []) }
      }
      case 'PreviewTeamMemberAdd': {
        const t = this.teamOf(cmd.data.team)
        const u = this.user(cmd.data.user)
        return {
          action: 'team_member_add',
          summary: `Add ${u.display_name} to ${t.name}`,
          affected: [u.display_name],
          step_up: false,
          apply: () => (t.members.push({ user: u.id, display_name: u.display_name }), []),
        }
      }
      case 'PreviewTeamMemberRemove': {
        const t = this.teamOf(cmd.data.team)
        const u = this.user(cmd.data.user)
        return {
          action: 'team_member_remove',
          summary: `Remove ${u.display_name} from ${t.name}`,
          affected: [u.display_name],
          step_up: false,
          apply: () => ((t.members = t.members.filter((m) => m.user !== u.id)), []),
        }
      }
      case 'PreviewGrantAdd': {
        const t = this.teamOf(cmd.data.team)
        const ws = this.ws(cmd.data.workspace)
        const { role, profiles } = cmd.data
        const scope = profiles.length === 0 ? 'the whole workspace' : `profiles ${profiles.join(', ')}`
        return {
          action: 'grant_add',
          summary: `Give ${t.name} ${role} on ${ws.slug} (${scope})`,
          affected: t.members.map((m) => m.display_name),
          step_up: true,
          apply: () => (t.grants.push({ id: this.id('grant'), workspace: ws.id, workspace_slug: ws.slug, role, profiles: [...profiles] }), []),
        }
      }
      case 'PreviewGrantRemove': {
        const t = this.teams.find((x) => x.grants.some((g) => g.id === cmd.data.grant))
        if (!t) return { action: 'grant_remove', summary: 'No such grant', affected: [], step_up: false, apply: () => [] }
        return {
          action: 'grant_remove',
          summary: `Remove a grant from ${t.name}`,
          affected: t.members.map((m) => m.display_name),
          step_up: false,
          apply: () => ((t.grants = t.grants.filter((g) => g.id !== cmd.data.grant)), []),
        }
      }
      case 'PreviewUserRole': {
        const u = this.user(cmd.data.user)
        const { role } = cmd.data
        return {
          action: 'user_role',
          summary: `Make ${u.display_name} ${role}`,
          affected: [u.display_name],
          step_up: true,
          apply: () => ((u.instance_role = role), []),
        }
      }
      case 'PreviewUserRemove': {
        const u = this.user(cmd.data.user)
        return {
          action: 'user_remove',
          summary: `Remove ${u.display_name}`,
          affected: [u.display_name],
          step_up: true,
          apply: () => ((u.disabled = true), []),
        }
      }
      case 'PreviewApiTokenCreate': {
        const ws = this.ws(cmd.data.workspace)
        const { name, actions } = cmd.data
        return {
          action: 'api_token_create',
          summary: `Create API token ${name} for ${ws.slug}`,
          affected: [],
          step_up: true,
          apply: () => {
            const token: ApiTokenInfo = {
              id: this.id('token'),
              name,
              owner: this.users[0]?.id ?? '',
              workspace: ws.id,
              workspace_slug: ws.slug,
              actions: [...actions],
              expires_at: '2026-11-26T12:00:00Z',
              last_used_at: null,
              revoked: false,
            }
            this.tokens.push(token)
            return [{ type: 'ApiTokenCreated', data: { token, secret: `kt_fake-secret-${token.id}` } }]
          },
        }
      }
      case 'PreviewApiTokenRevoke': {
        const token = this.tokens.find((t) => t.id === cmd.data.token)
        return {
          action: 'api_token_revoke',
          summary: `Revoke API token ${token?.name ?? cmd.data.token}`,
          affected: [],
          step_up: false,
          apply: () => (token && (token.revoked = true), []),
        }
      }
    }
  }
}

/** Installs governance on `backend` and returns it to script. */
export function governance(backend: FakeBackend, setup?: (g: FakeGovernance) => void): FakeGovernance {
  const g = new FakeGovernance()
  setup?.(g)
  return g.install(backend)
}

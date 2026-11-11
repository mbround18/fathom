/**
 * Spec 066's workspace repository and forge connection on the fake backend (T035), answering as
 * apps/keel/server does: `PreviewWorkspaceRepository` binds a repository on the workspace's
 * `default` project and makes it primary, `PreviewWorkspacePrimary` picks another,
 * `PreviewForgeConnection`/`PreviewForgeRemove` change the forge connection, `GetForgeConnection`
 * answers it (or `null`) and `ProbeForge` answers `probe`.
 *
 * It installs itself with `backend.on(...)` next to governance, whose workspaces it changes, and
 * the bindings land in `backend.repos`. IDs and timestamps are fixed, so stories stay deterministic.
 */
import type { BindingInfo, CredentialInfo, Event, ForgeConnectionInfo, WorkspaceInfo } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import type { FakeGovernance } from './fake-governance'
import { HEAD, NOW } from './fake-repos'

const EXPIRES = '2026-10-27T12:15:00Z'

export class FakeWorkspaceRepo {
  /** By workspace id. */
  readonly forges = new Map<string, ForgeConnectionInfo>()
  /** What `ProbeForge` answers. */
  probe: { ok: boolean; can_push: boolean; message: string | null } = { ok: true, can_push: true, message: null }

  private readonly previews = new Map<string, () => Event[]>()
  private seq = 0

  private readonly backend: FakeBackend
  private readonly g: FakeGovernance

  constructor(backend: FakeBackend, g: FakeGovernance) {
    this.backend = backend
    this.g = g
  }

  install(): this {
    const b = this.backend
    b.on('GetForgeConnection', (cmd) => [{ type: 'ForgeConnection', data: this.forges.get(cmd.data.workspace) ?? null }])
    b.on('ProbeForge', () => [{ type: 'ForgeProbe', data: this.probe }])
    b.on('PreviewWorkspaceRepository', (cmd) => {
      const ws = this.ws(cmd.data.workspace)
      const { spec } = cmd.data
      return this.preview('workspace_repository', `Bind ${spec.url} @ ${spec.branch} as ${ws.slug}'s repository.`, [spec.url], () => {
        const binding: BindingInfo = {
          id: this.id('binding'),
          project: `${ws.id}-default`,
          workspace: ws.id,
          ...spec,
          status: 'active',
          last_commit: HEAD,
          last_synced_at: NOW,
          last_sync_error: null,
          manifest_problems: [],
        }
        this.backend.repos.bindings.push(binding)
        ws.primary_binding = binding.id
        return [{ type: 'Binding', data: { binding, jobs: [] } }]
      })
    })
    b.on('PreviewWorkspacePrimary', (cmd) => {
      const ws = this.ws(cmd.data.workspace)
      const { binding } = cmd.data
      return this.preview('workspace_primary', `Make ${binding ?? 'nothing'} ${ws.slug}'s primary repository.`, [], () => {
        ws.primary_binding = binding
        return []
      })
    })
    b.on('PreviewForgeConnection', (cmd) => {
      const { workspace, provider, api_base, credential } = cmd.data
      const ws = this.ws(workspace)
      return this.preview('forge_connection', `Connect ${ws.slug} to ${provider}.`, [], () => {
        this.forges.set(workspace, {
          workspace,
          provider,
          api_base: api_base ?? 'https://git.example.com/api/v1',
          credential,
          probed_at: null,
          probe_error: null,
          created_at: NOW,
        })
        return []
      })
    })
    b.on('PreviewForgeRemove', (cmd) => {
      const ws = this.ws(cmd.data.workspace)
      return this.preview('forge_remove', `Disconnect ${ws.slug} from its forge.`, [], () => {
        this.forges.delete(ws.id)
        return []
      })
    })
    b.chain('Confirm', (cmd) => {
      const apply = this.previews.get(cmd.data.preview_id)
      if (!apply) return undefined
      this.previews.delete(cmd.data.preview_id)
      return [{ type: 'Confirmed', data: { preview_id: cmd.data.preview_id } }, ...apply()]
    })
    return this
  }

  /** A credential in `workspace`, listed by `ListCredentials`. */
  credential(workspace: string, name: string, kind: CredentialInfo['kind']): CredentialInfo {
    const c: CredentialInfo = { id: this.id('cred'), workspace, name, kind, fingerprint: 'sha256:0000', created_at: NOW, rotated_at: null, grants: [] }
    this.backend.repos.credentials.push(c)
    return c
  }

  private id(kind: string): string {
    return `wsr-${kind}-${String(++this.seq)}`
  }

  private ws(id: string): WorkspaceInfo {
    const ws = this.g.workspaces.find((w) => w.id === id)
    if (!ws) throw new Error(`no workspace ${id}`)
    return ws
  }

  private preview(action: string, summary: string, affected: string[], apply: () => Event[]): Event[] {
    const preview_id = this.id('preview')
    this.previews.set(preview_id, apply)
    return [{ type: 'Preview', data: { preview_id, action, summary, affected, step_up: false, expires_at: EXPIRES } }]
  }
}

/** Installs the workspace repository and forge fakes on `backend`, beside governance `g`. */
export function workspaceRepo(backend: FakeBackend, g: FakeGovernance): FakeWorkspaceRepo {
  return new FakeWorkspaceRepo(backend, g).install()
}

/**
 * Spec 066's repository catalog on the fake backend (T050), answering as apps/keel/server's
 * repository.rs does:
 *
 * - `GetCatalog` answers the workspace's `Catalog`, or refuses `no_primary_repository` (no primary
 *   binding) or `no_catalog` (not synced yet). `refusal` overrides both.
 * - `ListJobs` answers the primary binding's declared jobs, from the catalog.
 * - `DraftJob` answers a `JobDraft` derived the way keel-core's draft.rs does (R6).
 * - `PreviewHostAdd`, `PreviewSopsRecipient` and `PreviewJobPropose` preview; their `Confirm`
 *   answers `Confirmed`, then `HostAdded` or `ChangeProposed` with the forge's outcome
 *   (./fake-forge.ts).
 *
 * The catalog is modelled on crates/keel-core/tests/fixtures/ansible-repo. Its age keys are
 * synthetic and there is no ciphertext anywhere in it: the catalog only ever carries key names.
 */
import type {
  CatalogView,
  EnrollmentIssued,
  Event,
  HostDecrypt,
  WorkspaceInfo,
} from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import type { FakeForge } from './fake-forge'
import type { FakeGovernance } from './fake-governance'

import { AGE, draftYaml, fixtureCatalog, fixtureJobs, INVENTORY, inventoryDiff } from './catalog-fixtures'

export { AGE, draftYaml, fixtureCatalog, fixtureJobs, INVENTORY, inventoryDiff }

const EXPIRES = '2026-10-27T12:15:00Z'

type Pending = () => Event[]

export class FakeCatalog {
  /** Catalogs by workspace id; a workspace with a primary binding and none here has `no_catalog`. */
  readonly catalogs = new Map<string, CatalogView>()
  /** When set, `GetCatalog` refuses with it. */
  refusal: { code: string; message: string } | null = null
  /** Hosts added through `PreviewHostAdd`, in order. */
  readonly added: { workspace: string; inventory_name: string; groups: string[]; profiles: string[] }[] = []

  private readonly previews = new Map<string, Pending>()
  private seq = 0

  private readonly backend: FakeBackend
  private readonly g: FakeGovernance
  private readonly forge: FakeForge

  constructor(backend: FakeBackend, g: FakeGovernance, forge: FakeForge) {
    this.backend = backend
    this.g = g
    this.forge = forge
  }

  /** Gives `workspace` the fixture catalog on its primary binding, with these hosts' decrypt state. */
  synced(workspace: WorkspaceInfo, hosts: HostDecrypt[] = []): CatalogView {
    const binding = workspace.primary_binding
    if (binding === null) throw new Error(`${workspace.slug} has no primary repository`)
    const catalog = fixtureCatalog(binding, hosts)
    this.catalogs.set(workspace.id, catalog)
    return catalog
  }

  install(): this {
    const b = this.backend
    b.on('GetCatalog', (cmd) => {
      if (this.refusal) return [{ type: 'OpError', data: this.refusal }]
      const ws = this.ws(cmd.data.workspace)
      if (ws.primary_binding === null) {
        return [{ type: 'OpError', data: { code: 'no_primary_repository', message: `${ws.slug} has no primary repository.` } }]
      }
      const catalog = this.catalogs.get(ws.id)
      if (!catalog) return [{ type: 'OpError', data: { code: 'no_catalog', message: 'The repository has not been synced yet.' } }]
      return [{ type: 'Catalog', data: catalog }]
    })
    b.on('ListJobs', (cmd) => {
      const known = [...this.catalogs.values()].some((c) => c.binding === cmd.data.binding)
      return [{ type: 'Jobs', data: { binding: cmd.data.binding, items: known ? fixtureJobs(cmd.data.binding) : [] } }]
    })
    b.on('DraftJob', (cmd) => [{ type: 'JobDraft', data: draftYaml(cmd.data.playbook) }])
    b.on('PreviewHostAdd', (cmd) => {
      const { workspace, inventory_name, groups, profiles } = cmd.data
      const ws = this.ws(workspace)
      const catalog = this.catalogs.get(ws.id)
      const listed = catalog?.inventories.flatMap((i) => i.groups.flatMap((gr) => gr.hosts)) ?? []
      if (listed.includes(inventory_name)) {
        return [{ type: 'OpError', data: { code: 'host_exists', message: `${inventory_name} is already in ${INVENTORY}.` } }]
      }
      const group = groups[0] ?? 'all'
      const diff = inventoryDiff(group, inventory_name)
      const warnings = catalog?.secrets.files.some((f) => f.read_by.length > 0)
        ? [`${inventory_name} has no age key yet, so it cannot decrypt any sops file until its runner reports one.`]
        : []
      return this.preview(
        'host_add',
        `Enrol ${inventory_name} (${profiles.join(', ') || 'no profiles'}) and add it to ${groups.join(', ') || 'all'} in ${INVENTORY}.`,
        [INVENTORY, ...groups.map((x) => `group ${x}`)],
        warnings,
        () => {
          this.added.push({ workspace, inventory_name, groups, profiles })
          const enrollment: EnrollmentIssued = {
            id: this.id('enrollment'),
            workspace,
            inventory_name,
            profiles,
            expires_at: '2026-10-28T12:00:00Z',
            token: `keel-enroll-${inventory_name}-0123456789abcdef`,
            snippet: `keel-runner enroll --server https://keel.example.com --token keel-enroll-${inventory_name}-0123456789abcdef`,
            instructions: '',
            instructions_unavailable: false,
          }
          const change = this.forge.outcome(workspace, `add-host-${inventory_name}`, `Add ${inventory_name} to the inventory`, diff)
          return [{ type: 'HostAdded', data: { enrollment, change } }]
        },
      )
    })
    b.on('PreviewSopsRecipient', (cmd) => {
      const found = [...this.catalogs.entries()].flatMap(([ws, c]) => c.hosts.filter((h) => h.host === cmd.data.host).map((h) => ({ ws, h })))[0]
      if (!found?.h.age_recipient) {
        return [{ type: 'OpError', data: { code: 'no_age_recipient', message: 'The host has not reported an age key.' } }]
      }
      const { ws, h } = found
      const key = found.h.age_recipient
      const diff = ['--- a/.sops.yaml', '+++ b/.sops.yaml', '@@ -9,2 +9,2 @@', `-    age: ${AGE.keel},${AGE.kube}`, `+    age: ${AGE.keel},${AGE.kube},${key}`, ''].join('\n')
      return this.preview('sops_recipient', `Add ${h.inventory_name}'s age key to rule 1 in .sops.yaml.`, ['.sops.yaml'], [], () => [
        { type: 'ChangeProposed', data: this.forge.outcome(ws, `sops-${h.inventory_name}`, `Let ${h.inventory_name} decrypt its secrets`, diff) },
      ])
    })
    b.on('PreviewJobPropose', (cmd) => {
      const { workspace, playbook, name } = cmd.data
      const draft = draftYaml(playbook).yaml.replace(/^[^:]+:/, `${name}:`)
      const diff = ['--- a/.keel/jobs.yaml', '+++ b/.keel/jobs.yaml', '@@ -97,0 +98,13 @@', '+', ...draft.trimEnd().split('\n').map((l) => `+  ${l}`), ''].join('\n')
      return this.preview('job_propose', `Add job ${name} for ${playbook} to .keel/jobs.yaml.`, ['.keel/jobs.yaml'], [], () => [
        { type: 'ChangeProposed', data: this.forge.outcome(workspace, `job-${name}`, `Add job ${name}`, diff) },
      ])
    })
    b.chain('Confirm', (cmd) => {
      const apply = this.previews.get(cmd.data.preview_id)
      if (!apply) return undefined
      this.previews.delete(cmd.data.preview_id)
      return [{ type: 'Confirmed', data: { preview_id: cmd.data.preview_id } }, ...apply()]
    })
    return this
  }

  private id(kind: string): string {
    return `cat-${kind}-${String(++this.seq)}`
  }

  private ws(id: string): WorkspaceInfo {
    const ws = this.g.workspaces.find((w) => w.id === id)
    if (!ws) throw new Error(`no workspace ${id}`)
    return ws
  }

  private preview(action: string, summary: string, affected: string[], warnings: string[], apply: Pending): Event[] {
    const preview_id = this.id('preview')
    this.previews.set(preview_id, apply)
    return [{ type: 'Preview', data: { preview_id, action, summary, affected, step_up: false, expires_at: EXPIRES, warnings } }]
  }
}

/** Installs the catalog fake on `backend`, beside governance `g` and the forge fake. */
export function catalog(backend: FakeBackend, g: FakeGovernance, forge: FakeForge): FakeCatalog {
  return new FakeCatalog(backend, g, forge).install()
}

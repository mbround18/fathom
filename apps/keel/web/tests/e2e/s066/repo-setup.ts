/**
 * Shared set-up for spec 066's Repository, Secrets and Add host specs (T054): the workspace `ops`
 * with a primary repository bound and, unless asked otherwise, its catalog synced. Not a spec.
 */
import type { Page } from '@playwright/test'

import type { BindingInfo, HostDecrypt, WorkspaceInfo } from '../../../src/protocol/wire'
import { expect, fixtures, type FakeBackend } from '../../support/fake-backend'
import { catalog, type FakeCatalog } from '../../support/fake-catalog'
import { FakeForge } from '../../support/fake-forge'
import { governance, type FakeGovernance } from '../../support/fake-governance'
import { FakeHosts } from '../../support/fake-hosts'
import { HEAD, NOW } from '../../support/fake-repos'
import { workspaceRepo } from '../../support/fake-workspace-repo'

export const URL = 'https://git.example.com/ops/ansible.git'

export interface Repo {
  g: FakeGovernance
  ops: WorkspaceInfo
  forge: FakeForge
  cat: FakeCatalog
  hosts: FakeHosts
}

export interface Options {
  /** `owner` sees everything; `operator` and `viewer` hold that grant on `ops` alone. */
  role?: 'owner' | 'ws_admin' | 'operator' | 'viewer'
  /** Bind a primary repository (default true). */
  bound?: boolean
  /** Sync its catalog (default true), with these hosts' decrypt state. */
  synced?: boolean
  decrypts?: HostDecrypt[]
}

export function setUp(backend: FakeBackend, o: Options = {}): Repo {
  const role = o.role ?? 'owner'
  const g = governance(backend, (x) => {
    x.workspace('ops', 'Ops')
  })
  const ops = g.workspaces[0]
  if (!ops) throw new Error('no workspace')
  backend.me = fixtures.me({
    instance_role: role === 'owner' ? 'owner' : 'user',
    grants: role === 'owner' ? [] : [{ workspace: ops.id, workspace_slug: 'ops', role, profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  const hosts = new FakeHosts(backend)
  const forge = new FakeForge(workspaceRepo(backend, g))
  const cat = catalog(backend, g, forge)
  if (o.bound ?? true) {
    bind(backend, ops)
    if (o.synced ?? true) cat.synced(ops, o.decrypts ?? [])
  }
  return { g, ops, forge, cat, hosts }
}

function bind(backend: FakeBackend, ops: WorkspaceInfo): void {
  const binding: BindingInfo = {
    id: 'binding-ops',
    project: `${ops.id}-default`,
    workspace: ops.id,
    url: URL,
    branch: 'main',
    transport: 'https',
    credential: null,
    known_host: null,
    poll_interval: 300,
    allow_force_push: false,
    signed_mode: false,
    override_quorum: 1,
    status: 'active',
    last_commit: HEAD,
    last_synced_at: NOW,
    last_sync_error: null,
    manifest_problems: [],
  }
  backend.repos.bindings.push(binding)
  ops.primary_binding = binding.id
}

/** Confirms the open governance change titled `title` and closes it. */
export async function confirmChange(page: Page, title: string): Promise<void> {
  const dialog = page.getByRole('dialog', { name: title })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toHaveAttribute('data-change-dialog', 'done')
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(dialog).toBeHidden()
}

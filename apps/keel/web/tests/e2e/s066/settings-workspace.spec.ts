/**
 * Spec 066 T034/T035 (R10, contracts/ui.md "Settings → Workspace"): the current workspace's
 * repository, forge connection and danger zone, under Settings → Workspace. Owners and admins see
 * every tab, Workspace first; a ws_admin of the current workspace sees Workspace alone. Against the
 * fake backend.
 */
import type { Page } from '@playwright/test'

import type { BindingInfo } from '../../../src/protocol/wire'
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance, type FakeGovernance } from '../../support/fake-governance'
import { HEAD, NOW } from '../../support/fake-repos'
import { workspaceRepo, type FakeWorkspaceRepo } from '../../support/fake-workspace-repo'

const URL = 'https://git.example.com/ops/ansible.git'

function setUp(backend: FakeBackend, role: 'owner' | 'user' = 'owner'): { g: FakeGovernance; r: FakeWorkspaceRepo } {
  const g = governance(backend, (x) => {
    x.workspace('ops', 'Ops')
  })
  const ops = g.workspaces[0]
  if (!ops) throw new Error('no workspace')
  backend.me = fixtures.me({
    instance_role: role,
    grants: role === 'user' ? [{ workspace: ops.id, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }] : [],
    verification: fixtures.verification({ verified: true }),
  })
  return { g, r: workspaceRepo(backend, g) }
}

/** Gives `ops` a primary repository already bound. */
function bound(backend: FakeBackend, g: FakeGovernance): BindingInfo {
  const ops = g.workspaces[0]
  if (!ops) throw new Error('no workspace')
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
  return binding
}

async function confirmChange(page: Page, title: string): Promise<void> {
  const dialog = page.getByRole('dialog', { name: title })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toHaveAttribute('data-change-dialog', 'done')
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(dialog).toBeHidden()
}

test('Settings opens on Workspace, titled with the workspace, for an owner with every tab', async ({ page, backend }) => {
  setUp(backend)
  await page.goto('/settings')
  await expect(page).toHaveURL(/\/settings\/workspace\?ws=ops$/)
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Settings')
  await expect(page.getByRole('tab')).toHaveText(['Workspace', 'Enrollment', 'Tokens', 'Identity', 'System'])
  await expect(page.getByRole('heading', { name: 'Ops: Repository' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Ops: Forge connection' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Ops: Danger zone' })).toBeVisible()
})

test('the guided form sets up the repository, then the summary shows where it stands', async ({ page, backend }) => {
  const { g, r } = setUp(backend)
  const ops = g.workspaces[0]
  const token = r.credential(ops?.id ?? '', 'deploy-token', 'git_https_token')
  r.credential(ops?.id ?? '', 'deploy-key', 'git_ssh_key')
  await page.goto('/settings/workspace')

  const form = page.getByRole('form', { name: 'Set up repository' })
  await form.getByLabel('Repository URL').fill(URL)
  // Only the credentials the transport can use are offered.
  await expect(form.getByLabel('Credential', { exact: true }).locator('option')).toHaveText(['None (public)', 'deploy-token', 'New credential…'])
  await form.getByLabel('Transport').selectOption('ssh')
  await expect(form.getByLabel('Known host line')).toBeVisible()
  await expect(form.getByLabel('Credential', { exact: true }).locator('option')).toHaveText(['None (public)', 'deploy-key', 'New credential…'])
  await form.getByLabel('Transport').selectOption('https')
  await form.getByLabel('Credential', { exact: true }).selectOption({ label: 'deploy-token' })
  await form.getByRole('button', { name: 'Preview repository' }).click()

  const sent = await backend.waitForCmd('PreviewWorkspaceRepository')
  expect(sent.data.workspace).toBe(ops?.id)
  expect(sent.data.spec).toMatchObject({ url: URL, branch: 'main', transport: 'https', credential: token.id, poll_interval: 300 })
  await expect(page.getByRole('dialog', { name: 'Set up repository' })).toContainText(`Bind ${URL} @ main`)
  await confirmChange(page, 'Set up repository')

  const summary = page.locator('[data-repository]')
  await expect(summary).toContainText(URL)
  await expect(summary.locator('[data-head]')).toHaveText(HEAD.slice(0, 12))
  await expect(page.getByRole('form', { name: 'Set up repository' })).toHaveCount(0)
})

test('Sync now syncs the primary, and another can be chosen', async ({ page, backend }) => {
  const { g } = setUp(backend)
  const binding = bound(backend, g)
  await page.goto('/settings/workspace')
  await page.getByRole('button', { name: 'Sync now' }).click()
  expect((await backend.waitForCmd('SyncBinding')).data.binding).toBe(binding.id)

  await page.getByLabel('Change primary…').selectOption({ label: 'None' })
  expect((await backend.waitForCmd('PreviewWorkspacePrimary')).data).toEqual({ workspace: g.workspaces[0]?.id, binding: null })
  await confirmChange(page, 'Change primary repository')
  await expect(page.getByRole('form', { name: 'Set up repository' })).toBeVisible()
})

test('the forge connection is saved, tested and removed', async ({ page, backend }) => {
  const { g, r } = setUp(backend)
  bound(backend, g)
  const ops = g.workspaces[0]
  const token = r.credential(ops?.id ?? '', 'forge-bot', 'forge_token')
  r.probe = { ok: true, can_push: true, message: null }
  await page.goto('/settings/workspace')

  const form = page.getByRole('form', { name: 'Forge connection' })
  // The default API base is derived from the primary repository and shown, not filled in.
  await expect(form.getByLabel('API base')).toHaveAttribute('placeholder', 'https://git.example.com/api/v1')
  await expect(form.getByRole('button', { name: 'Test' })).toHaveCount(0)
  await form.getByLabel('Token', { exact: true }).selectOption({ label: 'forge-bot' })
  await form.getByRole('button', { name: 'Preview connection' }).click()
  expect((await backend.waitForCmd('PreviewForgeConnection')).data).toEqual({
    workspace: ops?.id,
    provider: 'forgejo',
    api_base: null,
    credential: token.id,
  })
  await confirmChange(page, 'Connect forge')

  await form.getByRole('button', { name: 'Test' }).click()
  await expect(form.locator('[data-probe="ok"]')).toContainText('the token can push')

  r.probe = { ok: false, can_push: false, message: 'The token was refused (401).' }
  await form.getByRole('button', { name: 'Test' }).click()
  await expect(form.locator('[data-probe="failed"]')).toHaveText('The token was refused (401).')

  await form.getByRole('button', { name: 'Remove' }).click()
  await confirmChange(page, 'Remove forge connection')
  await expect(form.getByRole('button', { name: 'Preview connection' })).toBeVisible()
  expect(r.forges.size).toBe(0)
})

test('a ws_admin of the current workspace sees Settings with the Workspace tab alone', async ({ page, backend }) => {
  setUp(backend, 'user')
  await page.goto('/hosts')
  await page.getByRole('navigation').getByRole('link', { name: 'Settings' }).click()
  await expect(page).toHaveURL(/\/settings\/workspace\?ws=ops$/)
  await expect(page.getByRole('tab')).toHaveText(['Workspace'])
  await page.goto('/settings/tokens')
  await expect(page).toHaveURL(/\/settings\/workspace\?ws=ops$/)
})

test('a user who administers no workspace sees no Settings entry', async ({ page, backend }) => {
  backend.me = fixtures.me({ instance_role: 'user' })
  governance(backend, (g) => {
    g.workspace('ops', 'Ops')
  })
  await page.goto('/hosts')
  await expect(page.getByRole('navigation').getByRole('link', { name: 'Settings' })).toHaveCount(0)
})

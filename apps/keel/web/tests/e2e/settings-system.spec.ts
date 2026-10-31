/**
 * Spec 060 US7 (T046): System Settings → System, owners only. An owner sets the System workspace
 * up, changes its profiles, binds a repository, opens Enroll on it and removes it; an admin sees
 * no System tab and is never sent there. Against the fake backend
 * (tests/support/fake-settings.ts; bindings from tests/support/fake-repos.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../support/fake-backend'
import { governance, type FakeGovernance } from '../support/fake-governance'
import { FakeHosts } from '../support/fake-hosts'
import { HEAD, NOW, PROJECT } from '../support/fake-repos'
import { FakeSettings, SYSTEM_WORKSPACE } from '../support/fake-settings'

const URL = 'https://git.example.com/keel/self.git'

function signIn(backend: FakeBackend, instance_role: 'owner' | 'admin'): { g: FakeGovernance; settings: FakeSettings } {
  backend.me = fixtures.me({ instance_role, grants: [], verification: fixtures.verification({ verified: true }) })
  const g = governance(backend, (gov) => {
    gov.workspace('ops')
  })
  new FakeHosts(backend)
  return { g, settings: new FakeSettings(backend) }
}

test('an owner sets up the System workspace, binds it, enrolls into it and removes it', async ({ page, backend }) => {
  const { g, settings } = signIn(backend, 'owner')
  await page.goto('/hosts')
  await page.getByRole('navigation').getByRole('link', { name: 'System Settings' }).click()
  await expect(page).toHaveURL(/\/settings\/enrollment$/)
  await page.getByRole('tab', { name: 'System' }).click()
  await expect(page).toHaveURL(/\/settings\/system$/)

  // Not configured yet: Set up creates it allowing `base`, through the confirm dialog.
  await expect(page.getByText('No System workspace')).toBeVisible()
  await page.getByRole('button', { name: 'Set up' }).click()
  expect((await backend.waitForCmd('PreviewSystemConfigure')).data).toEqual({ allowed_profiles: ['base'], version: 0 })
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-step-up]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  const configured = page.locator('[data-system="configured"]')
  await expect(configured).toBeVisible()
  expect(settings.system.configured).toBe(true)

  // Profiles: saved with the version they were edited from.
  const profiles = configured.getByRole('form', { name: 'System profiles' })
  await expect(profiles.getByLabel('Allowed profiles')).toHaveValue('base')
  await profiles.getByLabel('Allowed profiles').fill('base, monitoring base')
  await profiles.getByRole('button', { name: 'Save' }).click()
  expect((await backend.waitForCmd('PreviewSystemConfigure')).data).toEqual({ allowed_profiles: ['base', 'monitoring'], version: 1 })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(profiles.getByLabel('Allowed profiles')).toHaveValue('base, monitoring')

  // Bind the repository holding Keel's own playbooks, with the usual form.
  backend.repos.workspace = SYSTEM_WORKSPACE
  g.workspaces.push({
    id: SYSTEM_WORKSPACE,
    slug: 'keel-system',
    name: 'Keel system',
    created: NOW,
    archived: false,
    projects: [{ id: PROJECT, slug: 'keel', name: 'Keel' }],
    system: true,
  })
  const bindings = configured.getByRole('region', { name: 'System bindings' })
  await expect(bindings).toContainText('No repository is bound yet.')
  await bindings.getByRole('button', { name: 'New binding' }).click()
  const form = bindings.getByRole('form', { name: 'New binding' })
  // Its one project is picked already.
  await expect(form.getByLabel('Project')).toHaveValue(PROJECT)
  await form.getByLabel('Repository URL').fill(URL)
  await form.getByRole('button', { name: 'Preview binding' }).click()
  await expect(dialog).toContainText(`fetched ${HEAD.slice(0, 12)}`)
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(bindings.getByRole('list', { name: 'Bindings' }).locator('[data-binding]')).toContainText(URL)

  // Enroll a host into it: the dialog starts on the System workspace.
  const hosts = configured.getByRole('region', { name: 'System hosts' })
  await expect(hosts.locator('[data-system-hosts]')).toHaveText('0')
  await hosts.getByRole('button', { name: 'Enroll host' }).click()
  await expect(dialog.getByRole('form', { name: 'Enroll host' }).getByLabel('Workspace')).toHaveValue(SYSTEM_WORKSPACE)
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toHaveCount(0)

  // Remove: held back until the workspace's name is typed.
  const remove = configured.getByRole('form', { name: 'Remove the System workspace' })
  const button = remove.getByRole('button', { name: 'Remove' })
  await expect(button).toBeDisabled()
  await remove.getByLabel('Type keel-system to confirm').fill('keel-sys')
  await expect(button).toBeDisabled()
  await remove.getByLabel('Type keel-system to confirm').fill('keel-system')
  await button.click()
  await backend.waitForCmd('PreviewSystemRemove')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(page.getByText('No System workspace')).toBeVisible()
  expect(settings.system.configured).toBe(false)
})

test('a profiles save after someone else changed it shows the conflict banner', async ({ page, backend }) => {
  const { settings } = signIn(backend, 'owner')
  settings.system = { configured: true, workspace: SYSTEM_WORKSPACE, allowed_profiles: ['base'], version: 3, bindings: [], hosts: ['host-1'] }
  await page.goto('/settings/system')
  const configured = page.locator('[data-system="configured"]')
  await expect(configured.locator('[data-system-hosts]')).toHaveText('1')

  settings.systemChangedElsewhere()
  const profiles = configured.getByRole('form', { name: 'System profiles' })
  await profiles.getByLabel('Allowed profiles').fill('base web')
  await profiles.getByRole('button', { name: 'Save' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Dismiss' }).click()
  await expect(page.locator('[data-conflict]')).toContainText('the System workspace')
  expect(settings.system.allowed_profiles).toEqual(['base'])
})

test('an admin sees the System tab without the System workspace', async ({ page, backend }) => {
  signIn(backend, 'admin')
  await page.goto('/settings/system')
  // Spec 061 FR-024: the tab opens to admins, with the running build and nothing of spec 060's.
  await expect(page).toHaveURL(/\/settings\/system$/)
  const tabs = page.getByRole('tablist', { name: 'Settings' }).getByRole('tab')
  await expect(tabs).toHaveText(['Enrollment', 'Tokens', 'Identity', 'System'])
  await expect(page.locator('[data-build-panel]')).toBeVisible()
  await expect(page.locator('[data-system]')).toHaveCount(0)
  expect(backend.received.some((c) => c.type === 'GetSystem')).toBe(false)
})

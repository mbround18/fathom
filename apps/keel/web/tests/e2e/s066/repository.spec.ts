/**
 * Spec 066 T051/T052 (contracts/ui.md "Repository page", "Draft job"): the current workspace's
 * catalog in five tabs, selection by `?item=`, the stale warning, the empty states, and Draft job
 * with and without a forge connection. Against the fake backend.
 */
import { expect, test } from '../../support/fake-backend'
import { confirmChange, setUp } from './repo-setup'

test('Repository is in the nav and opens on Playbooks with the catalog', async ({ page, backend }) => {
  setUp(backend)
  await page.goto('/')
  await page.getByRole('link', { name: 'Repository' }).click()
  await expect(page).toHaveURL(/\/repository/)
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Repository')
  await expect(page.getByRole('tab')).toHaveText(['Playbooks', 'Roles', 'Inventory', 'Secrets', 'Jobs'])
  const tab = page.getByTestId('repo-tab-playbooks')
  await expect(tab.getByTestId('catalog-item')).toHaveCount(7)
  await expect(tab.locator('[data-testid="catalog-item"][data-item="playbooks/keel-server.yaml"]')).toBeVisible()
  await expect(page.getByText('1 problems')).toBeVisible()
})

test('each tab loads its own items and the tab is in the address', async ({ page, backend }) => {
  setUp(backend)
  await page.goto('/repository')
  const cases: [string, string, number][] = [
    ['Roles', 'roles', 8],
    ['Inventory', 'inventory', 5],
    ['Secrets', 'secrets', 3],
    ['Jobs', 'jobs', 6],
  ]
  for (const [label, tab, count] of cases) {
    await page.getByRole('tab', { name: label }).click()
    await expect(page).toHaveURL(new RegExp(`tab=${tab}`))
    await expect(page.getByTestId(`repo-tab-${tab}`).getByTestId('catalog-item')).toHaveCount(count)
  }
})

test('?item= opens the detail, and its links move between tabs', async ({ page, backend }) => {
  setUp(backend)
  await page.goto('/repository?tab=playbooks&item=playbooks/k3s-master.yaml')
  const detail = page.locator('[data-detail-pane]')
  await expect(detail).toHaveAttribute('aria-label', 'Playbook playbooks/k3s-master.yaml')
  await expect(detail).toContainText('secrets/k3s_servers.sops.yaml')
  await detail.getByRole('button', { name: 'sops', exact: true }).click()
  await expect(page).toHaveURL(/tab=roles&item=sops/)
  await expect(page.locator('[data-detail-pane]')).toHaveAttribute('aria-label', 'Role sops')

  await page.locator('[data-detail-pane]').getByRole('button', { name: 'Close' }).click()
  await expect(page.locator('[data-detail-pane]')).toHaveCount(0)
  await expect(page).not.toHaveURL(/item=/)
})

test('a stale catalog says its last build failed', async ({ page, backend }) => {
  const { ops, cat } = setUp(backend)
  const c = cat.catalogs.get(ops.id)
  if (!c) throw new Error('no catalog')
  cat.catalogs.set(ops.id, { ...c, stale: true, error: 'roles/base/meta/main.yml: not YAML' })
  await page.goto('/repository')
  await expect(page.locator('[data-catalog-stale]')).toContainText('The last build failed: roles/base/meta/main.yml: not YAML')
  await expect(page.getByTestId('repo-tab-playbooks').getByTestId('catalog-item')).toHaveCount(7)
})

test('with no primary repository an admin is sent to set one up', async ({ page, backend }) => {
  setUp(backend, { bound: false })
  await page.goto('/repository')
  await expect(page.locator('[data-empty-page="repository"]')).toContainText('Set up the repository')
  await page.getByRole('link', { name: 'Set up the repository', exact: true }).click()
  await expect(page).toHaveURL(/\/settings\/workspace\?ws=ops$/)
})

test('with no primary repository a viewer is told who can set one up', async ({ page, backend }) => {
  setUp(backend, { bound: false, role: 'viewer' })
  await page.goto('/repository')
  await expect(page.locator('[data-empty-page="repository"]')).toContainText('No repository yet')
  await expect(page.locator('[data-who-can]')).toContainText('A workspace admin')
  await expect(page.getByRole('link', { name: 'Set up the repository', exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Add host' })).toHaveCount(0)
})

test('before the first sync the page says so', async ({ page, backend }) => {
  setUp(backend, { synced: false })
  await page.goto('/repository')
  await expect(page.locator('[data-empty-page="repository-not-synced"]')).toContainText('Not synced yet')
  await expect(page.getByRole('tab')).toHaveCount(0)
})

test.describe('Draft job', () => {
  test.beforeEach(async ({ context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  })

  test('without a forge, the entry is shown to copy by hand', async ({ page, backend }) => {
    setUp(backend)
    await page.goto('/repository?tab=playbooks&item=playbooks/cleanup.yaml')
    await page.locator('[data-draft-job]').click()
    const dialog = page.getByRole('dialog', { name: 'Draft job' })
    await expect(dialog.getByTestId('draft-yaml')).toContainText('playbook: playbooks/cleanup.yaml')
    await expect(dialog.getByRole('list', { name: 'Notes' })).toContainText('No manifest profile is named "cleanup".')
    await expect(dialog.locator('[data-no-forge]')).toBeVisible()
    await expect(dialog.getByRole('button', { name: 'Propose as pull request' })).toHaveCount(0)

    await dialog.getByRole('button', { name: 'Copy job entry' }).click()
    expect(await page.evaluate(() => navigator.clipboard.readText())).toContain('cleanup:\n  playbook: playbooks/cleanup.yaml')
    await dialog.getByRole('button', { name: 'Close' }).last().click()
    await expect(dialog).toBeHidden()
  })

  test('with a forge, Propose opens a pull request', async ({ page, backend }) => {
    const { ops, forge } = setUp(backend)
    forge.connect(ops.id)
    await page.goto('/repository?tab=playbooks&item=playbooks/cleanup.yaml')
    await page.locator('[data-draft-job]').click()
    const dialog = page.getByRole('dialog', { name: 'Draft job' })
    await expect(dialog.getByTestId('draft-yaml')).toBeVisible()
    await expect(dialog.getByLabel('Job name')).toHaveValue('cleanup')
    await dialog.getByLabel('Job name').fill('Bad Name')
    await expect(dialog.getByRole('button', { name: 'Propose as pull request' })).toBeDisabled()
    await dialog.getByLabel('Job name').fill('nightly-cleanup')
    await dialog.getByRole('button', { name: 'Propose as pull request' }).click()

    expect((await backend.waitForCmd('PreviewJobPropose')).data).toEqual({ workspace: ops.id, playbook: 'playbooks/cleanup.yaml', name: 'nightly-cleanup' })
    await confirmChange(page, 'Propose job')
    const outcome = dialog.getByTestId('change-outcome')
    await expect(outcome).toHaveAttribute('data-outcome', 'opened')
    await expect(outcome.getByRole('link')).toHaveAttribute('href', 'https://git.example.com/ops/ansible/pulls/41')
    expect(forge.changes.map((c) => c.title)).toEqual(['Add job nightly-cleanup'])
  })

  test('when the forge refuses, the patch is offered to apply by hand', async ({ page, backend }) => {
    const { ops, forge } = setUp(backend)
    forge.connect(ops.id).mode = 'failed'
    await page.goto('/repository?tab=playbooks&item=playbooks/cleanup.yaml')
    await page.locator('[data-draft-job]').click()
    const dialog = page.getByRole('dialog', { name: 'Draft job' })
    await dialog.getByRole('button', { name: 'Propose as pull request' }).click()
    await confirmChange(page, 'Propose job')
    const outcome = dialog.getByTestId('change-outcome')
    await expect(outcome).toHaveAttribute('data-outcome', 'failed')
    await expect(outcome.getByRole('alert')).toContainText('The forge refused the change')
    await expect(outcome).toContainText('+++ b/.keel/jobs.yaml')
  })
})

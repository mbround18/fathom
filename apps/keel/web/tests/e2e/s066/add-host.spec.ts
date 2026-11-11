/**
 * Spec 066 T053 (contracts/ui.md "Add host", "Hosts", FR-014 to FR-017): Add host from Hosts and
 * from a Repository → Inventory group, its review and result (enrolment snippet and inventory
 * change), a host already listed, each host's inventory status, and its age key with Propose sops
 * recipient for a workspace admin. Against the fake backend.
 */
import type { Page } from '@playwright/test'

import { AGE } from '../../support/catalog-fixtures'
import { expect, test } from '../../support/fake-backend'
import { hostInfo } from '../../support/fake-hosts'
import { confirmChange, setUp } from './repo-setup'

const SNIPPET = (name: string) => `keel-runner enroll --server https://keel.example.com --token keel-enroll-${name}-0123456789abcdef`

async function review(page: Page, name: string): Promise<void> {
  const dialog = page.getByRole('dialog', { name: 'Add host' })
  await dialog.getByLabel('Inventory name').fill(name)
  await dialog.getByRole('button', { name: 'Review' }).click()
}

test('Add host from Hosts adds it to the inventory and hands over the enrolment command', async ({ page, backend }) => {
  const { ops, cat } = setUp(backend, { role: 'operator' })
  await page.goto('/hosts')
  await page.getByTestId('add-host').click()
  const dialog = page.getByRole('dialog', { name: 'Add host' })
  await expect(dialog).toHaveAttribute('data-add-host-dialog', 'form')
  const groups = dialog.getByRole('group', { name: 'Groups' })
  await expect(groups.getByRole('button')).toHaveText(['compose_hosts', 'gameservers', 'k3s_servers', 'keel_servers'])
  await groups.getByRole('button', { name: 'compose_hosts' }).click()
  await expect(groups.getByRole('button', { name: 'compose_hosts' })).toHaveAttribute('aria-pressed', 'true')
  await review(page, 'delta')

  expect((await backend.waitForCmd('PreviewHostAdd')).data).toEqual({
    workspace: ops.id,
    inventory_name: 'delta',
    groups: ['compose_hosts'],
    profiles: ['base'],
    inventory: null,
  })
  await expect(page.getByRole('dialog', { name: 'Add host delta' })).toContainText('add it to compose_hosts in inventory/hosts.yml')
  // The server's warnings show before confirming (FR-018).
  await expect(page.getByRole('list', { name: 'Warnings' })).toContainText('delta has no age key yet')
  await confirmChange(page, 'Add host delta')

  const result = page.getByRole('dialog', { name: 'Host added' })
  await expect(result).toHaveAttribute('data-add-host-dialog', 'result')
  await expect(result.locator('[data-copy-block="enroll-snippet"]')).toContainText(SNIPPET('delta'))
  // No forge connection: the inventory change is a patch to apply by hand.
  await expect(result.getByTestId('change-outcome')).toHaveAttribute('data-outcome', 'manual')
  await expect(result.getByTestId('change-outcome')).toContainText('+    delta:')
  expect(cat.added).toEqual([{ workspace: ops.id, inventory_name: 'delta', groups: ['compose_hosts'], profiles: ['base'] }])
  await result.getByRole('button', { name: 'Done' }).last().click()
  await expect(result).toBeHidden()
})

test('Add host here, from an inventory group, starts with that group and opens a pull request', async ({ page, backend }) => {
  const { ops, forge } = setUp(backend, { role: 'operator' })
  forge.connect(ops.id)
  await page.goto('/repository?tab=inventory&item=inventory/hosts.yml%23gameservers')
  await expect(page.locator('[data-detail-pane]')).toHaveAttribute('aria-label', 'Group gameservers')
  await page.locator('[data-add-host-here]').click()
  const dialog = page.getByRole('dialog', { name: 'Add host' })
  await expect(dialog.getByRole('button', { name: 'gameservers' })).toHaveAttribute('aria-pressed', 'true')
  await expect(dialog.getByRole('button', { name: 'compose_hosts' })).toHaveAttribute('aria-pressed', 'false')
  await review(page, 'factorio')
  await confirmChange(page, 'Add host factorio')

  const outcome = page.getByRole('dialog', { name: 'Host added' }).getByTestId('change-outcome')
  await expect(outcome).toHaveAttribute('data-outcome', 'opened')
  await expect(outcome.getByRole('link')).toHaveAttribute('href', 'https://git.example.com/ops/ansible/pulls/41')
  expect(forge.changes.map((c) => c.branch)).toEqual(['keel/add-host-factorio-1'])
})

test('a viewer gets neither Add host here nor Add host', async ({ page, backend }) => {
  setUp(backend, { role: 'viewer' })
  await page.goto('/repository?tab=inventory&item=inventory/hosts.yml%23gameservers')
  await expect(page.locator('[data-detail-pane]')).toBeVisible()
  await expect(page.locator('[data-add-host-here]')).toHaveCount(0)
  await page.goto('/hosts')
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await expect(page.getByTestId('add-host')).toHaveCount(0)
})

test('a host the inventory already lists is refused', async ({ page, backend }) => {
  const { cat } = setUp(backend)
  await page.goto('/repository?tab=inventory&item=inventory/hosts.yml%23all')
  await expect(page.locator('[data-add-host-here]')).toHaveCount(0)
  await page.getByRole('button', { name: 'Add host' }).click()
  await review(page, 'alpha')
  const change = page.getByRole('dialog', { name: 'Add host alpha' })
  await expect(change).toHaveAttribute('data-change-dialog', 'refused')
  await expect(change).toContainText('alpha is already in inventory/hosts.yml.')
  expect(cat.added).toEqual([])
})

test("each host's inventory status shows on Hosts, with a badge while it is pending", async ({ page, backend }) => {
  const { ops, hosts } = setUp(backend, { role: 'operator' })
  hosts.hosts = [
    hostInfo({ id: 'host-alpha', workspace: ops.id, inventory_name: 'alpha', inventory_status: 'listed' }),
    hostInfo({ id: 'host-delta', workspace: ops.id, inventory_name: 'delta', inventory_status: 'pending_inventory' }),
  ]
  await page.goto('/hosts')
  const table = page.getByRole('table', { name: 'Hosts' })
  const delta = table.locator('[data-inventory-status="pending_inventory"]')
  await expect(delta).toContainText('delta')
  await expect(delta.locator('[data-pending-inventory]')).toHaveText('Pending inventory')
  await expect(table.locator('[data-inventory-status="listed"] [data-pending-inventory]')).toHaveCount(0)
})

test.describe('age key', () => {
  const decrypts = [
    { host: 'host-alpha', inventory_name: 'alpha', age_recipient: AGE.alpha, decrypts: [] },
    { host: 'host-kube1', inventory_name: 'kube1', age_recipient: AGE.kube, decrypts: [] },
  ]

  function withHosts(backend: Parameters<typeof setUp>[0], role: 'ws_admin' | 'operator') {
    const repo = setUp(backend, { role, decrypts })
    repo.hosts.hosts = [
      hostInfo({ id: 'host-alpha', workspace: repo.ops.id, inventory_name: 'alpha', inventory_status: 'listed', age_recipient: AGE.alpha }),
      hostInfo({ id: 'host-kube1', workspace: repo.ops.id, inventory_name: 'kube1', inventory_status: 'listed', age_recipient: AGE.kube }),
      hostInfo({ id: 'host-new', workspace: repo.ops.id, inventory_name: 'new', inventory_status: 'pending_inventory' }),
    ]
    return repo
  }

  test('a key no rule names can be proposed as a sops recipient by a workspace admin', async ({ page, backend }) => {
    withHosts(backend, 'ws_admin')
    await page.goto('/hosts')
    await page.getByRole('button', { name: 'Show alpha' }).click()
    const key = page.locator('[data-age-key]')
    await expect(key).toHaveAttribute('data-age-key', 'reported')
    await page.locator('[data-propose-recipient]').click()
    expect((await backend.waitForCmd('PreviewSopsRecipient')).data).toEqual({ host: 'host-alpha' })
    await confirmChange(page, 'Add alpha as a sops recipient')
    await expect(key.getByTestId('change-outcome')).toHaveAttribute('data-outcome', 'manual')
    await expect(key.getByTestId('change-outcome')).toContainText(AGE.alpha)
    await expect(page.locator('[data-propose-recipient]')).toHaveCount(0)
  })

  test('a key a rule already names, or no key yet, offers nothing to propose', async ({ page, backend }) => {
    withHosts(backend, 'ws_admin')
    await page.goto('/hosts')
    await page.getByRole('button', { name: 'Show kube1' }).click()
    await expect(page.locator('[data-age-key]')).toContainText('A .sops.yaml rule already names this key.')
    await expect(page.locator('[data-propose-recipient]')).toHaveCount(0)
    await page.getByRole('button', { name: 'Show new' }).click()
    await expect(page.locator('[data-age-key]')).toHaveAttribute('data-age-key', 'none')
    await expect(page.locator('[data-age-key]')).toContainText('Not reported yet.')
    await expect(page.locator('[data-pending-inventory]')).toHaveCount(2)
  })

  test('an operator sees the key but cannot propose it', async ({ page, backend }) => {
    withHosts(backend, 'operator')
    await page.goto('/hosts')
    await page.getByRole('button', { name: 'Show alpha' }).click()
    await expect(page.locator('[data-age-key]')).toHaveAttribute('data-age-key', 'reported')
    await expect(page.locator('[data-propose-recipient]')).toHaveCount(0)
  })
})

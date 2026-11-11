/**
 * Spec 066 T051 (contracts/ui.md "Repository → Secrets", FR-016): the `.sops.yaml` rules, each
 * sops file's recipients and drift, which hosts can decrypt it, and the `sops updatekeys` fix.
 * The catalog carries key names only, never ciphertext. Against the fake backend.
 */
import { AGE } from '../../support/catalog-fixtures'
import { expect, test } from '../../support/fake-backend'
import { setUp } from './repo-setup'

const K3S = 'secrets/k3s_servers.sops.yaml'
const KEEL = 'secrets/keel_servers.sops.yaml'

const DECRYPTS = [
  { host: 'host-keel', inventory_name: 'keel', age_recipient: AGE.keel, decrypts: [K3S, KEEL] },
  { host: 'host-kube1', inventory_name: 'kube1', age_recipient: AGE.kube, decrypts: [] },
  { host: 'host-steam', inventory_name: 'steam', age_recipient: null, decrypts: [] },
]

test('the rules and files are listed, with drift and unreadable files flagged', async ({ page, backend }) => {
  setUp(backend, { decrypts: DECRYPTS })
  await page.goto('/repository?tab=secrets')
  const tab = page.getByTestId('repo-tab-secrets')
  await expect(tab.locator('[data-sops-rule]')).toHaveCount(2)
  await expect(tab.getByTestId('catalog-item')).toHaveCount(3)
  const k3s = tab.locator(`[data-testid="catalog-item"][data-item="${K3S}"]`)
  await expect(k3s.locator('[data-drift]')).toHaveText('recipients out of date')
  await expect(tab.locator('[data-testid="catalog-item"] [data-drift]')).toHaveCount(1)
  await expect(tab.locator('[data-testid="catalog-item"][data-item="secrets/broken.sops.yaml"]')).toContainText('unreadable')
})

test("each host's decrypt state is shown per file", async ({ page, backend }) => {
  setUp(backend, { decrypts: DECRYPTS })
  await page.goto('/repository?tab=secrets')
  const row = page.locator(`[data-testid="catalog-item"][data-item="${K3S}"]`)
  await expect(row.locator('[data-decrypt="ok"]')).toHaveAttribute('data-host', 'keel')
  await expect(row.locator('[data-decrypt="cannot"]')).toHaveAttribute('data-host', 'kube1')
  await expect(row.locator('[data-decrypt="unknown"]')).toHaveAttribute('data-host', 'steam')
  await expect(row.locator('[data-decrypt="unknown"]')).toContainText('no key reported')
})

test('a drifted file shows what is missing and the updatekeys command', async ({ page, backend }) => {
  setUp(backend, { decrypts: DECRYPTS })
  await page.goto(`/repository?tab=secrets&item=${K3S}`)
  const detail = page.locator('[data-detail-pane]')
  await expect(detail).toHaveAttribute('aria-label', `Secrets file ${K3S}`)
  await expect(detail.getByRole('list', { name: 'Missing recipients' })).toContainText(AGE.kube)
  await expect(detail.getByRole('list', { name: 'Keys' })).toContainText('k3s_token')
  await expect(detail).toContainText(`sops updatekeys ${K3S}`)
  await detail.getByRole('button', { name: 'k3s-master' }).click()
  await expect(page).toHaveURL(/tab=jobs&item=k3s-master/)
})

test('a file without drift has no updatekeys command', async ({ page, backend }) => {
  setUp(backend, { decrypts: DECRYPTS })
  await page.goto(`/repository?tab=secrets&item=${KEEL}`)
  const detail = page.locator('[data-detail-pane]')
  await expect(detail.getByRole('list', { name: 'Recipients' })).toContainText(AGE.operator)
  await expect(detail).not.toContainText('sops updatekeys')
})

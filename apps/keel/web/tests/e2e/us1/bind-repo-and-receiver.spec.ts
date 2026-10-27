/**
 * US1 (spec 055 T043): bind a repository (the preview shows the test fetch), create a receiver
 * (its secret is shown once), bind them, and see deliveries with their decisions. Against the
 * fake backend (tests/support/fake-repos.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { HEAD, PROJECT, WORKSPACE } from '../../support/fake-repos'

const URL = 'https://git.example.com/ops/infra.git'

function admin(backend: FakeBackend): void {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true, enrolled: true, at: '2026-10-27T11:59:00Z' }),
  })
}

test('bind a repository, create and bind a receiver, see its deliveries', async ({ page, backend }) => {
  admin(backend)
  await page.goto('/bindings')
  await expect(page.getByRole('heading', { name: 'Bindings', level: 1 })).toBeVisible()
  await expect(page.getByText('No repository is bound yet.')).toBeVisible()

  // Create a binding: the preview reports the test fetch before anything is stored.
  await page.getByRole('button', { name: 'New binding' }).click()
  const form = page.getByRole('form', { name: 'New binding' })
  await form.getByLabel('Project id').fill(PROJECT)
  await form.getByLabel('Repository URL').fill(URL)
  await form.getByRole('button', { name: 'Preview binding' }).click()
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText(`fetched ${HEAD.slice(0, 12)}`)
  expect(backend.repos.bindings).toHaveLength(0)
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()

  const table = page.getByRole('table', { name: 'Bindings' })
  await expect(table.getByRole('button', { name: URL })).toBeVisible()
  await expect(table).toContainText(HEAD.slice(0, 12))
  // The confirm answered the binding, so its jobs show.
  await expect(page.getByRole('table', { name: 'Jobs' })).toContainText('playbooks/web.yaml')

  // Create a receiver: step-up is asked for, and the secret is shown once.
  await page.getByRole('navigation', { name: 'Sections' }).getByRole('link', { name: 'Receivers' }).click()
  await expect(page.getByRole('heading', { name: 'Receivers', level: 1 })).toBeVisible()
  const create = page.getByRole('form', { name: 'New receiver' })
  await create.getByLabel('Name').fill('github-main')
  await create.getByRole('button', { name: 'Preview receiver' }).click()
  await expect(dialog).toContainText('asks for your passkey')
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  const shown = page.getByRole('region', { name: 'Receiver secret' })
  const secret = backend.repos.secrets[0] ?? ''
  expect(secret).not.toBe('')
  await expect(shown).toContainText(secret)
  await expect(shown).toContainText('/hooks/')
  await shown.getByRole('button', { name: 'I have copied it' }).click()
  await expect(shown).toBeHidden()
  await expect(page.locator('body')).not.toContainText(secret)

  // Bind the receiver to the binding.
  const row = page.getByRole('listitem', { name: 'Receiver github-main' })
  await row.getByLabel('Bind to').selectOption({ label: `${URL} @ main` })
  await row.getByRole('button', { name: 'Bind' }).click()
  await expect(dialog).toContainText('Let github-main sync')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(row.getByRole('list', { name: 'Bound bindings' })).toContainText(URL)

  // Deliveries arrive live, with their decisions.
  const receiver = backend.repos.receivers[0]?.id ?? ''
  backend.push(backend.repos.deliver(receiver, 'gh-delivery-1'))
  backend.push(backend.repos.deliver(receiver, 'gh-delivery-2', 'bad_signature'))
  const deliveries = page.getByRole('table', { name: 'Deliveries' })
  await expect(deliveries.locator('tr[data-decision="accepted"]')).toContainText('gh-delivery-1')
  await expect(deliveries.locator('tr[data-decision="accepted"]')).toContainText('synced')
  await expect(deliveries.locator('tr[data-decision="refused"]')).toContainText('refused: bad_signature')

  // A reload lists what was recorded, and the secret is nowhere.
  await page.reload()
  await expect(page.getByRole('table', { name: 'Deliveries' }).locator('tbody tr')).toHaveCount(2)
  await expect(page.locator('body')).not.toContainText(secret)
})

test('a repository bound twice is refused with the reason', async ({ page, backend }) => {
  admin(backend)
  await page.goto('/bindings')
  for (const attempt of [1, 2]) {
    if (attempt === 1) await page.getByRole('button', { name: 'New binding' }).click()
    const form = page.getByRole('form', { name: 'New binding' })
    await form.getByLabel('Project id').fill(PROJECT)
    await form.getByLabel('Repository URL').fill(URL)
    await form.getByRole('button', { name: 'Preview binding' }).click()
    if (attempt === 1) {
      await page.getByRole('dialog').getByRole('button', { name: 'Confirm' }).click()
      await expect(page.getByRole('dialog')).toBeHidden()
    }
  }
  await expect(page.getByRole('alert')).toContainText('already bound')
})

test('without a workspace the page says so', async ({ page }) => {
  await page.goto('/bindings')
  await expect(page.getByText('You have no workspace yet.')).toBeVisible()
})

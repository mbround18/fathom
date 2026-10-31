/**
 * Spec 063 T015 (US1, FR-015): every value someone has to paste elsewhere has its own Copy
 * button, copies exactly, and keeps its "shown once" warning beside it. The enrol token and
 * snippet, and a new receiver's URL and secret. Against the fake backend.
 */
import type { Locator, Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { FakeHosts } from '../../support/fake-hosts'
import { WORKSPACE } from '../../support/fake-repos'

function admin(backend: FakeBackend): FakeHosts {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true, enrolled: true, at: '2026-10-27T11:59:00Z' }),
  })
  backend.on('ListWorkspaces', () => [
    {
      type: 'Workspaces',
      data: {
        items: [
          { id: WORKSPACE, slug: 'ops', name: 'Ops', created: '2026-10-27T12:00:00Z', archived: false, projects: [], system: false },
        ],
      },
    },
  ])
  return new FakeHosts(backend)
}

/** Copies `label` from `scope` and checks the clipboard holds `value`. */
async function copies(page: Page, scope: Locator, label: string, value: string): Promise<void> {
  const button = scope.getByRole('button', { name: `Copy ${label}`, exact: true })
  await expect(button).toHaveCount(1)
  await button.click()
  await expect(button).toHaveText('Copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(value)
}

test.beforeEach(async ({ context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
})

test('the enrol token and snippet each copy exactly', async ({ page, backend }) => {
  const hosts = admin(backend)
  await page.goto('/hosts')
  await page.locator('[data-empty-action]').getByRole('button', { name: 'Enroll host' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Inventory name').fill('web-1')
  await dialog.getByRole('button', { name: 'Continue' }).click()
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  const result = dialog.getByRole('region', { name: 'Enrolment token' })
  await expect(result).toContainText('Keel will not show it again')
  const token = hosts.tokens[0]?.token ?? 'missing'
  await expect(result.locator('[data-copy="enroll-token"]')).toHaveAttribute('data-masked', 'true')
  await copies(page, result, 'token', token)
  const snippet = (await result.locator('[data-copy="enroll-snippet"]').textContent()) ?? ''
  expect(snippet).toContain(token)
  await copies(page, result, 'snippet', snippet)
  await expect(result).toContainText('Keel will not show it again')
})

test('a new receiver URL and secret each copy exactly', async ({ page, backend }) => {
  admin(backend)
  await page.goto('/receivers')
  const form = page.getByRole('form', { name: 'New receiver' })
  await form.getByLabel('Name').fill('github-main')
  await form.getByRole('button', { name: 'Preview receiver' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm' }).click()

  const shown = page.getByRole('region', { name: 'Receiver secret' })
  await expect(shown).toContainText('Keel will not show it again')
  const secret = backend.repos.secrets[0] ?? 'missing'
  const url = (await shown.locator('[data-copy="receiver-url"]').textContent()) ?? ''
  expect(url).toContain('/hooks/')
  await copies(page, shown, 'payload url', url)

  // The secret is masked until revealed; Copy takes the real value either way.
  const value = shown.locator('[data-copy="receiver-secret"]')
  await expect(value).toHaveAttribute('data-masked', 'true')
  await expect(value).not.toContainText(secret)
  await copies(page, shown, 'secret', secret)
  await shown.getByRole('button', { name: 'Reveal secret' }).click()
  await expect(value).toHaveText(secret)
  await expect(shown).toContainText('Keel will not show it again')
})

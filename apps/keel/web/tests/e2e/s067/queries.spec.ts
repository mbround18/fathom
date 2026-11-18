/**
 * Spec 067 US5 (T043): the host inspector's Queries panel. An answered query opens to its JSON,
 * a refused one says why, and a runner without `query-v1` shows "runner too old" instead of the
 * form. Viewers read the list but get no form. Against the fake backend
 * (tests/support/fake-queries.ts).
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { FakeQueries } from '../../support/fake-queries'

import { scene } from './scene'

async function openHost(page: Page, name: string) {
  await page.goto('/hosts')
  await page.getByRole('button', { name: `Show ${name}` }).click()
  return page.getByRole('region', { name: `Host ${name}` }).getByRole('region', { name: 'Queries' })
}

test('an answered query lists newest first and opens to its answer', async ({ page, backend }) => {
  scene(backend)
  const queries = new FakeQueries(backend)
  queries.capability['host-0003'] = true
  queries.seed('host-0003', { kind: 'facts', answer: { kernel: '6.8.0' } })

  const panel = await openHost(page, 'lab-1')
  await expect(panel.locator('li[data-query-outcome]')).toHaveCount(1)

  const form = panel.getByRole('form', { name: 'Ask the host' })
  await form.getByLabel('Kind').selectOption('unit_state')
  // A bad unit never leaves the browser.
  await form.getByLabel('Unit', { exact: true }).fill('nginx;reboot')
  await form.getByRole('button', { name: 'Ask' }).click()
  await expect(form).toContainText('Letters, digits and @ . _ : - only.')
  await expect(form.getByLabel('Unit', { exact: true })).toHaveAttribute('aria-invalid', 'true')

  await form.getByLabel('Unit', { exact: true }).fill('nginx.service')
  await form.getByRole('button', { name: 'Ask' }).click()
  const asked = await backend.waitForCmd('AskHost')
  expect(asked.data).toEqual({ host: 'host-0003', kind: 'unit_state', params: { unit: 'nginx.service' } })

  const rows = panel.locator('li[data-query-outcome]')
  await expect(rows).toHaveCount(2)
  await expect(rows.first()).toHaveAttribute('data-query-outcome', 'waiting')
  await expect(rows.first()).toContainText('Unit state nginx.service')

  queries.settle('answered', { answer: { active: 'active', sub: 'running' } })
  await expect(rows.first()).toHaveAttribute('data-query-outcome', 'answered')
  await expect(rows.first()).toContainText('Ada Admin')
  await rows.first().getByText('Answer', { exact: true }).click()
  await expect(rows.first().locator('pre')).toContainText('"sub": "running"')
  await expect(rows.nth(1)).toContainText('Facts')
})

test('a refused query says why', async ({ page, backend }) => {
  scene(backend)
  const queries = new FakeQueries(backend)
  queries.capability['host-0003'] = true

  const panel = await openHost(page, 'lab-1')
  await expect(panel.getByText('Nothing asked yet.')).toBeVisible()
  const form = panel.getByRole('form', { name: 'Ask the host' })
  await form.getByLabel('Kind').selectOption('path_free')
  await form.getByLabel('Path', { exact: true }).fill('/srv/data')
  await form.getByRole('button', { name: 'Ask' }).click()
  await backend.waitForCmd('AskHost')
  await expect(panel.locator('li[data-query-outcome="waiting"]')).toHaveCount(1)

  queries.settle('refused', { refusal: 'not_allowed' })
  const row = panel.locator('li[data-query-outcome="refused"]')
  await expect(row).toContainText('Free space /srv/data')
  await expect(row).toContainText('refused: not allowed by the host’s queries.toml')
  await expect(row.getByText('Answer', { exact: true })).toHaveCount(0)
})

test('a runner too old for queries shows that instead of the form', async ({ page, backend }) => {
  scene(backend)
  const queries = new FakeQueries(backend)
  queries.capability['host-0004'] = false
  queries.seed('host-0004', { outcome: 'no_capability', answer: null })

  const panel = await openHost(page, 'lab-2')
  await expect(panel.locator('[data-queries="too-old"]')).toHaveText('Runner too old for queries. Upgrade keel-runner on this host to ask it.')
  await expect(panel.getByRole('form', { name: 'Ask the host' })).toHaveCount(0)
  await expect(panel.locator('li[data-query-outcome="no_capability"]')).toContainText('runner too old for queries')
})

test('viewers read the answers but cannot ask', async ({ page, backend }) => {
  scene(backend, { role: 'viewer' })
  const queries = new FakeQueries(backend)
  queries.capability['host-0003'] = true
  queries.seed('host-0003')

  const panel = await openHost(page, 'lab-1')
  await expect(panel.locator('li[data-query-outcome="answered"]')).toHaveCount(1)
  await expect(panel.getByRole('form', { name: 'Ask the host' })).toHaveCount(0)
})

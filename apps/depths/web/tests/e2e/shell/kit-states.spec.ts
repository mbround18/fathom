/**
 * Spec 065 T023: Depths' list reads fail the kit's way. Each list, refused once, says
 * "Failed to load {what}." with the reason and a Retry that loads it; a page that throws while
 * rendering shows the kit's crash state, and the navigation still works.
 */
import type { Page } from '@playwright/test'

import type { AuditEntry, Event } from '../../../src/protocol/generated'
import { expect, fixtures as f, test, type CmdType, type FakeBackend } from '../../support/fake-backend'

const DENIED: Event[] = [{ type: 'Denied', data: { reason: 'not_permitted' } }]

/** `type` is refused while the returned switch is on; turning it off falls through to the fake. */
function refuse(backend: FakeBackend, type: CmdType): { failing: boolean } {
  const state = { failing: true }
  backend.on(type, () => (state.failing ? DENIED : undefined))
  return state
}

/** A GET to `path` answers 500 while the returned switch is on. */
async function failHttp(page: Page, path: string): Promise<{ failing: boolean }> {
  const state = { failing: true }
  await page.route(`**${path}`, (route) =>
    state.failing && route.request().method() === 'GET'
      ? route.fulfill({ status: 500, json: { error: 'error' } })
      : route.fallback(),
  )
  return state
}

const failed = (page: Page, what: string) =>
  page.locator('[data-pane-state="error"]').filter({ hasText: `Failed to load ${what}.` })

/** The failure says what and why; Retry, once the server answers, shows the list. */
async function retries(page: Page, what: string, reason: string, state: { failing: boolean }): Promise<void> {
  const pane = failed(page, what)
  await expect(pane).toBeVisible()
  await expect(pane).toContainText(reason)
  state.failing = false
  await pane.getByRole('button', { name: 'Retry' }).click()
  await expect(failed(page, what)).toHaveCount(0)
}

const NOT_PERMITTED = 'You are not permitted to do this. (not_permitted)'

test('the stack list', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const state = refuse(backend, 'ListStacks')
  await page.goto('/')
  await retries(page, 'stacks', NOT_PERMITTED, state)
  await expect(page.locator('[data-stack="web"]').first()).toBeVisible()
})

test("a stack's mounts and a folder", async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'readme.txt', 'hi\n')

  const mounts = refuse(backend, 'ListMounts')
  await page.goto('/stacks/web/files')
  await retries(page, 'mounts', 'You are not allowed to browse these files. (not_permitted)', mounts)
  await expect(page.locator('[data-pane="mounts"]')).toContainText('web_data')

  const folder = refuse(backend, 'ListDir')
  await page.goto(`/stacks/web/files/${data}`)
  await retries(page, 'this folder', 'You are not allowed to browse these files. (not_permitted)', folder)
  await expect(page.locator('[data-entry="readme.txt"]')).toBeVisible()
})

test('your sessions and networks', async ({ page, backend }) => {
  backend.sessionList = [f.sessionView('abcd', { current: true })]
  backend.networks = [f.networkView('home-net')]
  const sessions = await failHttp(page, '/api/v1/sessions')
  const networks = await failHttp(page, '/api/v1/networks')
  await page.goto('/account/sessions')
  await retries(page, 'your sessions', 'The server answered 500. (http_500)', sessions)
  await retries(page, 'your networks', 'The server answered 500. (http_500)', networks)
})

test('your passkeys', async ({ page, backend }) => {
  backend.passkeys.enable({
    items: [{ id: 'cGhvbmU', name: 'Phone', created: '2026-10-01T09:00:00Z', last_used: null }],
    verified: true,
  })
  const state = refuse(backend, 'ListPasskeys')
  await page.goto('/account/sessions')
  await retries(page, 'your passkeys', NOT_PERMITTED, state)
  await expect(page.getByTestId('passkeys-panel').locator('[data-passkey]')).toHaveCount(1)
})

test('the users', async ({ page, backend }) => {
  backend.users = { users: [f.user('u1')], pending: [] }
  const state = refuse(backend, 'GetUsers')
  await page.goto('/admin/users')
  await retries(page, 'users', NOT_PERMITTED, state)
  await expect(page.getByTestId('user-count')).toBeVisible()
})

test('the audit log', async ({ page, backend }) => {
  const state = refuse(backend, 'GetAudit')
  await page.goto('/admin/audit')
  await retries(page, 'the audit log', NOT_PERMITTED, state)
  await expect(page.getByRole('table', { name: 'Audit entries' }).locator('[data-audit-row]').first()).toBeVisible()
})

test('the updates', async ({ page, backend }) => {
  backend.setUpdates(f.updatesView())
  const state = refuse(backend, 'ListUpdates')
  await page.goto('/admin/updates')
  await retries(page, 'updates', NOT_PERMITTED, state)
  await expect(page.getByTestId('updates-schedule')).toBeVisible()
})

test('a page that crashes shows the crash state, and the navigation still works', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  // An entry without its outcome: the audit row cannot render it.
  const broken = { at: '2026-10-01T09:00:00Z', actor: { provider: 'github', subject: '1', username: 'ada' }, action: 'pull' }
  backend.audit = [broken as unknown as AuditEntry]
  await page.goto('/admin/audit')
  await expect(page.locator('[data-pane-state="error"]')).toContainText('Something went wrong showing this page.')

  await page.getByRole('navigation').getByRole('link', { name: 'Stacks' }).first().click()
  await expect(page.getByRole('heading', { name: 'Stacks', level: 1 })).toBeVisible()
  await expect(page.locator('[data-stack="web"]').first()).toBeVisible()
})

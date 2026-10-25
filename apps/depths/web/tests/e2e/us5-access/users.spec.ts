/**
 * US5 users and grants (T069): pending people, what a user on one stack can and cannot reach, a
 * grant change taking effect at the next command, and the admin's users view (owners locked,
 * pre-added ids, missing stacks).
 *
 * The fake authorises each command against `backend.me` as it is at that moment, as the server
 * does (FR-032, FR-033), so a test changes a grant by assigning `backend.me` mid-test.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const BOB = { provider: 'github' as const, subject: '2002', username: 'bob', email: null }

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([f.stack('x', { has_env: true }), f.stack('y')], {
    foreign: [{ name: 'adhoc', working_dir: '/home/ada/adhoc', containers: [f.container({ name: 'adhoc-app-1', project: 'adhoc' })], elsewhere_of: null }],
    standalone: [f.container({ name: 'loose-redis', image: 'redis:8' })],
  })
  backend.users = {
    users: [
      f.user('github:1001', { username: 'ada', role: 'admin', owner: true }),
      f.user('github:2002', { username: 'bob', stacks: ['x', 'gone'] }),
    ],
    pending: [f.user('discord:3003', { username: 'cleo', role: 'pending' })],
  }
}

const row = (page: Page, id: string) => page.locator(`[data-user="${id}"]`)

test('a pending person gets the no-access page and waits in the admin list', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.pending({ identity: { ...BOB, provider: 'discord', subject: '3003', username: 'cleo' } })
  await page.goto('/')
  await expect(page.getByRole('heading', { name: "You don't currently have access" })).toBeVisible()
  await expect(page.locator('[data-stack]')).toHaveCount(0)
  // Nothing about the host on the no-access page.
  await expect(page.locator('body')).not.toContainText('/srv/stacks')

  backend.me = sessions.admin()
  await page.goto('/admin/users')
  const pending = page.getByRole('region', { name: /Waiting for access/ })
  await expect(pending.locator('[data-user="discord:3003"]')).toContainText('cleo')

  // Admit cleo as a user on x.
  await row(page, 'discord:3003').getByRole('button', { name: 'Admit cleo' }).click()
  const form = page.getByRole('form', { name: 'Access for cleo' })
  await form.getByRole('checkbox', { name: 'x' }).check()
  await form.getByRole('button', { name: 'Save' }).click()
  expect((await backend.waitForCmd('PutUser')).data).toEqual({ id: 'discord:3003', role: 'user', stacks: ['x'] })
  const users = page.getByRole('region', { name: /People with access/ })
  await expect(users.locator('[data-user="discord:3003"]')).toContainText('x')
  await expect(pending.locator('[data-user]')).toHaveCount(0)
})

test('a user on x sees x only: no other stack, no foreign group, no admin views', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['x'], { identity: BOB })
  await page.goto('/')
  await expect(page.locator('[data-stack]')).toHaveCount(1)
  await expect(page.locator('[data-stack="x"]')).toBeVisible()
  await expect(page.locator('[data-foreign], [data-standalone]')).toHaveCount(0)
  await expect(page.getByRole('navigation', { name: 'Admin' })).toHaveCount(0)

  // The admin pages do not exist for a user, and ask the server nothing.
  for (const path of ['/admin/users', '/admin/audit']) {
    await page.goto(path)
    await expect(page.getByRole('heading', { name: 'Not found' })).toBeVisible()
  }
  expect(backend.received.filter((c) => c.type === 'GetUsers' || c.type === 'GetAudit')).toEqual([])
})

test('a user on x can run actions, open a shell and edit x masked; reveal and admin-only ops are not offered', async ({ page, backend }) => {
  seed(backend)
  backend.env = { x: 'DB_PASSWORD=hunter2\n' }
  backend.me = sessions.user(['x'], { identity: BOB })
  await page.goto('/stacks/x')
  await expect(page.getByRole('heading', { name: 'x', level: 1 })).toBeVisible()

  for (const action of ['up', 'stop', 'restart', 'down', 'pull', 'update']) {
    await expect(page.locator(`[aria-label="Stack actions"] [data-action="${action}"]`)).toBeVisible()
  }
  await expect(page.locator('[data-action="delete"]')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Shell in x-app-1' })).toBeVisible()

  // `down` is offered, but without removing volumes or images.
  await page.locator('[aria-label="Stack actions"] [data-action="down"]').click()
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-step]').first()).toBeVisible()
  await expect(dialog.getByRole('switch', { name: /remove named volumes/i })).toHaveCount(0)
  await page.keyboard.press('Escape')

  // Revealing .env is admin-only (FR-021): the user edits it masked, with no reveal offered.
  await page.locator('[data-action="edit"]').click()
  await expect(page.locator('[data-code-editor="env"] .cm-content')).toContainText('DB_PASSWORD=********')
  await expect(page.locator('[data-action="reveal-env"]')).toHaveCount(0)
  await expect(page.getByText('hunter2')).toHaveCount(0)
  await expect(page.locator('[data-action="save"]')).toBeVisible()

  await page.goto('/new')
  await expect(page.getByText('Only admins can create stacks.')).toBeVisible()
})

test('a user reaching for another stack is denied, and the attempt is audited', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['x'], { identity: BOB })
  // The UI never links to y; the URL is typed by hand.
  await page.goto('/stacks/y')
  await expect(page.getByText('This stack is not available to you.')).toBeVisible()
  expect((await backend.waitForCmd('GetStack')).data).toEqual({ name: 'y' })
  await expect.poll(() => backend.audit.map((e) => [e.action, e.stack, e.actor.username])).toEqual([['denied', 'y', 'bob']])

  backend.me = sessions.admin()
  await page.goto('/admin/audit')
  const entry = page.locator('[data-audit-row][data-action="denied"]')
  await expect(entry).toContainText('bob')
  await expect(entry).toContainText('y')
  await expect(entry).toContainText('Denied: not_permitted')
})

test("a grant change applies at the user's next command; removing the last stack ends the session", async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['x'], { identity: BOB })
  await page.goto('/stacks/x')
  await expect(page.getByRole('heading', { name: 'x', level: 1 })).toBeVisible()

  // An admin adds y: the next command sees it, with no new sign-in.
  backend.me = sessions.user(['x', 'y'], { identity: BOB })
  await page.getByRole('link', { name: 'Stacks' }).first().click()
  await page.evaluate(() => {
    history.pushState(null, '', '/stacks/y')
    dispatchEvent(new Event('depths:navigate'))
  })
  await expect(page.getByRole('heading', { name: 'y', level: 1 })).toBeVisible()
  expect(backend.http.filter((h) => h.path === '/api/v1/me')).toHaveLength(1)

  // Then every stack is taken away: the next command closes the socket, and the session with it.
  backend.me = sessions.user([], { identity: BOB })
  await page.locator('[aria-label="Stack actions"] [data-action="restart"]').click()
  await expect(page.getByRole('heading', { name: "You don't currently have access" })).toBeVisible()
})

test('the owner is locked; an admin pre-adds an id and sees missing stacks', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/users')

  const owner = row(page, 'github:1001')
  await expect(owner.locator('[data-badge="owner"]')).toHaveText('Owner')
  // Locked: nothing to change. Viewing their sessions and networks (spec 052) is still offered.
  await expect(owner.getByRole('button', { name: /^(Edit|Remove|Admit) / })).toHaveCount(0)
  await expect(owner.getByRole('button')).toHaveCount(1)

  // An assigned stack with no directory is kept, and marked.
  await expect(row(page, 'github:2002').locator('[data-stack="gone"][data-missing]')).toContainText('(missing)')
  await expect(row(page, 'github:2002').locator('[data-stack="x"]:not([data-missing])')).toBeVisible()

  // A display name is not an id.
  const add = page.getByRole('form', { name: 'Add a person by id' })
  await add.getByRole('textbox', { name: 'Provider id' }).fill('dana')
  await add.getByRole('button', { name: 'Add' }).click()
  await expect(add).toContainText('Use a provider id')
  expect(backend.received.filter((c) => c.type === 'PutUser')).toEqual([])

  await add.getByRole('textbox', { name: 'Provider id' }).fill('discord:4004')
  await add.getByRole('button', { name: 'Add' }).click()
  expect((await backend.waitForCmd('PutUser')).data).toEqual({ id: 'discord:4004', role: 'user', stacks: [] })
  const dana = row(page, 'discord:4004')
  await expect(dana).toContainText('Never signed in')
  await dana.getByRole('button', { name: 'Edit discord:4004' }).click()
  await page.getByRole('form', { name: 'Access for discord:4004' }).getByRole('checkbox', { name: 'y' }).check()
  await page.getByRole('form', { name: 'Access for discord:4004' }).getByRole('button', { name: 'Save' }).click()
  await expect(dana.locator('[data-stack="y"]')).toBeVisible()

  // Remove asks first, then goes.
  await row(page, 'github:2002').getByRole('button', { name: 'Remove bob' }).click()
  await row(page, 'github:2002').getByRole('button', { name: 'Remove', exact: true }).click()
  expect((await backend.waitForCmd('RemoveUser')).data).toEqual({ id: 'github:2002' })
  await expect(row(page, 'github:2002')).toHaveCount(0)

  // Were the server to refuse an owner change, the reason shows.
  backend.deny('PutUser', 'owner')
  await dana.getByRole('button', { name: 'Edit discord:4004' }).click()
  await page.getByRole('form', { name: 'Access for discord:4004' }).getByRole('button', { name: 'Save' }).click()
  await expect(page.locator('[data-banner="denied"]')).toContainText('Owners cannot be changed here.')
})

test('another admin changing users shows up live', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/users')
  await expect(row(page, 'github:2002')).toBeVisible()
  backend.push({ type: 'UsersChanged', data: { users: backend.users.users, pending: [] } })
  await expect(page.getByRole('region', { name: /Waiting for access/ })).toContainText('No one is waiting.')
})

test('the users view fits a phone', async ({ page, backend }) => {
  seed(backend)
  await page.setViewportSize({ width: 390, height: 800 })
  await page.goto('/admin/users')
  await expect(row(page, 'github:2002')).toBeVisible()
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
  expect(overflow).toBeLessThanOrEqual(0)
})

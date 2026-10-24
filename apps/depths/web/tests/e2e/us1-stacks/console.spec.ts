/**
 * The stack page, console first: a live log console for one container (the first running one by
 * default, or every container), buttons that pop it out into tabs, the compose file and `.env`
 * beside it on wide screens and below it on narrow ones. Revealing `.env` is admin-only (FR-021).
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const T = (s: number) => `2026-10-23T12:00:${String(s).padStart(2, '0')}Z`

function seed(backend: FakeBackend, over: Parameters<typeof f.stack>[1] = {}): void {
  const web = f.stack('web', {
    services: [
      {
        name: 'app',
        image: 'nginx:1.29',
        containers: [
          f.container({ name: 'web-app-1', project: 'web', service: 'app', state: 'exited', status: 'Exited (0) 1 hour ago' }),
          f.container({ name: 'web-app-2', project: 'web', service: 'app' }),
        ],
      },
      { name: 'db', image: 'postgres:18', containers: [f.container({ name: 'web-db-1', project: 'web', service: 'db', image: 'postgres:18' })] },
    ],
    status: 'partial',
    running_services: 2,
    total_services: 2,
    ...over,
  })
  backend.stacks = f.stackList([web])
  backend.logs['web-app-2'] = [`${T(1)} app two ready`]
  backend.logs['web-db-1'] = [`${T(1)} database system is ready`]
  backend.logs.web = [`app | ${T(1)} every container`]
}

const consoleBox = (page: Page) => page.locator('[data-console]')
const consoleLogs = (page: Page) => consoleBox(page).getByTestId('log-view')

async function openStack(page: Page): Promise<void> {
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
}

test('the console streams the first running container, and switches source on a pick', async ({ page, backend }) => {
  seed(backend)
  await openStack(page)

  const first = await backend.waitForCmd('OpenLogs')
  expect(first.data).toEqual({ target: { kind: 'container', container: 'web-app-2' }, tail: 1000 })
  await expect(consoleBox(page)).toHaveAttribute('data-console-target', 'web-app-2')
  await expect(consoleLogs(page)).toContainText('app two ready')
  await expect(page.locator('[data-container="web-app-2"] [data-action="console-pick"]')).toHaveAttribute('aria-pressed', 'true')
  // The stopped container is listed with its state, and the others' details stay on the page.
  await expect(page.locator('[data-container="web-app-1"] [data-field="state"]')).toHaveText('exited')
  await expect(page.locator('[data-container="web-db-1"] [data-field="image"]')).toHaveText('postgres:18')

  // Another container: the old stream closes and the new one opens.
  await page.locator('[data-container="web-db-1"] [data-action="console-pick"]').click()
  await backend.waitForCmd('CloseStream')
  expect((await backend.waitForCmd('OpenLogs')).data.target).toEqual({ kind: 'container', container: 'web-db-1' })
  await expect(consoleLogs(page)).toContainText('database system is ready')
  await expect(consoleLogs(page)).not.toContainText('app two ready')
  await expect.poll(() => backend.openStreams().length).toBe(1)

  // Every container: the stack's own log target.
  await page.locator('[data-action="console-pick-all"]').click()
  expect((await backend.waitForCmd('OpenLogs')).data.target).toEqual({ kind: 'stack', stack: 'web' })
  await expect(consoleLogs(page)).toContainText('every container')
  await expect(consoleBox(page)).toHaveAttribute('data-console-target', 'stack:web')
})

test('with nothing running the console shows every container', async ({ page, backend }) => {
  seed(backend)
  for (const s of backend.stacks.stacks[0]?.services ?? []) for (const c of s.containers) c.state = 'exited'
  await openStack(page)
  expect((await backend.waitForCmd('OpenLogs')).data.target).toEqual({ kind: 'stack', stack: 'web' })
  await expect(page.locator('[data-action="console-pick-all"]')).toHaveAttribute('aria-pressed', 'true')
})

test('the console pops out into tabs, and leaving the page closes its stream', async ({ page, backend }) => {
  seed(backend)
  await openStack(page)
  await expect(consoleLogs(page)).toContainText('app two ready')

  await consoleBox(page).getByRole('button', { name: 'Open the logs of web-app-2 in a tab' }).click()
  await consoleBox(page).getByRole('button', { name: 'Open a shell in web-app-2 in a tab' }).click()
  await consoleBox(page).getByRole('button', { name: 'Open the stats of web-app-2 in a tab' }).click()
  await expect(page.getByRole('tab')).toHaveCount(3)
  expect((await backend.waitForCmd('OpenShell')).data.container).toBe('web-app-2')
  await expect.poll(() => backend.openStreams().length).toBe(3)

  // Leaving the stack: the console's stream closes, the tabs' stay.
  await page.getByRole('link', { name: 'Stacks' }).first().click()
  await expect(consoleBox(page)).toHaveCount(0)
  await backend.waitForCmd('CloseStream')
  await expect.poll(() => backend.openStreams().length).toBe(2)
})

test('with the shell off, the console offers no shell', async ({ page, backend }) => {
  seed(backend)
  backend.me = f.me({ features: { shell: false, edit: true } })
  await openStack(page)
  await expect(consoleBox(page).locator('[data-action="console-pop-logs"]')).toBeVisible()
  await expect(consoleBox(page).locator('[data-action="console-pop-shell"]')).toHaveCount(0)
})

test.describe('wide screens', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('the console is in the middle and the files beside it, compose above .env', async ({ page, backend }) => {
    seed(backend)
    await openStack(page)
    const con = await consoleBox(page).boundingBox()
    const compose = await page.locator('#compose-title').boundingBox()
    const env = await page.locator('#env-title').boundingBox()
    expect(con && compose && env).toBeTruthy()
    if (!con || !compose || !env) return
    expect(compose.x).toBeGreaterThanOrEqual(con.x + con.width)
    expect(env.y).toBeGreaterThan(compose.y)
    // The console fills the height left under the header and the services.
    expect(con.height).toBeGreaterThan(250)
  })
})

test.describe('narrow screens', () => {
  test.use({ viewport: { width: 360, height: 740 } })

  test('header, services and console, then compose, then .env, with no sideways scroll', async ({ page, backend }) => {
    seed(backend, { has_env: true })
    await openStack(page)
    await expect(consoleLogs(page)).toContainText('app two ready')
    const con = await consoleBox(page).boundingBox()
    const compose = await page.locator('#compose-title').boundingBox()
    const env = await page.locator('#env-title').boundingBox()
    if (!con || !compose || !env) throw new Error('missing a part of the page')
    expect(compose.y).toBeGreaterThan(con.y + con.height - 1)
    expect(env.y).toBeGreaterThan(compose.y)
    expect(con.height).toBeGreaterThan(200)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  })
})

test.describe('.env on the stack page', () => {
  test('an admin reveals the real values, told it is audited, and they hide again', async ({ page, backend }) => {
    seed(backend, { has_env: true })
    backend.env.web = 'PORT=8080\nDB_PASSWORD=hunter2\n'
    await openStack(page)
    const env = page.locator('[data-code-view="env"]')
    await expect(env).toContainText('DB_PASSWORD=********')
    await expect(page.locator('[data-field="reveal-audited"]')).toHaveText('Revealing is recorded in the audit log.')

    await page.locator('[data-action="reveal-env"]').click()
    expect((await backend.waitForCmd('RevealEnv')).data).toEqual({ name: 'web' })
    await expect(env).toContainText('DB_PASSWORD=hunter2')
    await expect(page.locator('[data-field="env-note"]')).toContainText('masked again')

    await page.locator('[data-action="hide-env"]').click()
    await expect(env).toContainText('DB_PASSWORD=********')
    await expect(page.getByText('hunter2')).toHaveCount(0)
  })

  test('a revealed .env is masked again on leaving the page', async ({ page, backend }) => {
    seed(backend, { has_env: true })
    backend.env.web = 'PORT=8080\nDB_PASSWORD=hunter2\n'
    await openStack(page)
    await page.locator('[data-action="reveal-env"]').click()
    await expect(page.locator('[data-code-view="env"]')).toContainText('DB_PASSWORD=hunter2')
    await page.getByRole('link', { name: 'Stacks' }).first().click()
    await expect(page.locator('[data-code-view="env"]')).toHaveCount(0)
    await page.locator('[data-stack="web"]').getByRole('link', { name: 'web' }).click()
    await expect(page.locator('[data-code-view="env"]')).toContainText('DB_PASSWORD=********')
    await expect(page.getByText('hunter2')).toHaveCount(0)
  })

  test('a user sees .env masked, with no way to reveal it', async ({ page, backend }) => {
    seed(backend, { has_env: true })
    backend.me = sessions.user(['web'])
    backend.env.web = 'PORT=8080\nDB_PASSWORD=hunter2\n'
    await openStack(page)
    await expect(page.locator('[data-code-view="env"]')).toContainText('DB_PASSWORD=********')
    await expect(page.locator('[data-action="reveal-env"]')).toHaveCount(0)
    await expect(page.locator('[data-field="reveal-audited"]')).toHaveCount(0)
  })
})

/**
 * US1: sign in and see every stack (T045), against the fake backend.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const row = (page: Page, name: string) => page.locator(`[data-stack="${name}"]`)

/** Three stacks (running, partial, stopped), one never deployed, one invalid, one unreadable. */
function seed(backend: FakeBackend): void {
  const jobs = f.stack('jobs', {
    // A one-shot `migrate` that exited 0 is completed: the server counts it out, the stack is running.
    services: [
      { name: 'app', image: 'nginx:1.29', containers: [f.container({ name: 'jobs-app-1', project: 'jobs', service: 'app' })] },
      {
        name: 'migrate',
        image: 'busybox',
        containers: [
          f.container({ name: 'jobs-migrate-1', project: 'jobs', service: 'migrate', state: 'exited', status: 'Exited (0) 1 hour ago' }),
        ],
      },
    ],
    running_services: 1,
    total_services: 1,
  })
  backend.stacks = f.stackList(
    [
      f.stack('web'),
      f.stack('cache', { status: 'partial', running_services: 1, total_services: 2 }),
      f.stack('db', { status: 'stopped' }),
      f.stack('fresh', { status: 'not_deployed' }),
      f.stack('broken', { status: 'invalid', valid_name: true }),
      f.stack('locked', { status: 'invalid', read_error: 'Permission denied (os error 13)' }),
      jobs,
    ],
    {
      foreign: [
        { name: 'adhoc', working_dir: '/home/ada/adhoc', containers: [f.container({ name: 'adhoc-app-1', project: 'adhoc' })], elsewhere_of: null },
        { name: 'webtest', working_dir: '/srv/stacks/web', containers: [f.container({ name: 'webtest-app-1', project: 'webtest' })], elsewhere_of: 'web' },
      ],
      standalone: [f.container({ name: 'loose-redis', image: 'redis:8' })],
      warnings: ['broken: yaml: line 3: mapping values are not allowed in this context'],
    },
  )
}

test('a visitor who is not signed in is sent to sign in and sees no stack', async ({ page, backend }) => {
  seed(backend)
  backend.me = null
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  await expect(page.getByText('web', { exact: true })).toHaveCount(0)
  expect(backend.sockets).toHaveLength(0)
})

test('the list shows every status, as text, with service counts', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  const expected: [string, string][] = [
    ['web', 'Running'],
    ['cache', 'Partly running'],
    ['db', 'Stopped'],
    ['fresh', 'Not deployed'],
    ['broken', 'Invalid'],
  ]
  for (const [name, label] of expected) {
    await expect(row(page, name).locator('[data-status]')).toHaveText(label)
  }
  await expect(row(page, 'cache').locator('[data-field="services"]')).toHaveText('1/2 services up')
  // The completed one-shot does not make `jobs` partial.
  await expect(row(page, 'jobs').locator('[data-status]')).toHaveText('Running')
  await expect(row(page, 'jobs').locator('[data-field="services"]')).toHaveText('1/1 services up')
})

test('invalid and unreadable stacks say why and offer no actions', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await expect(row(page, 'broken').locator('[data-note="invalid"]')).toHaveText('The compose file does not parse')
  // The parser's own message, verbatim, from the server's warnings.
  await expect(page.locator('[data-banner="warnings"]')).toContainText('yaml: line 3: mapping values are not allowed')
  await expect(row(page, 'locked').locator('[data-note="read"]')).toHaveText('Cannot read: Permission denied (os error 13)')
  for (const name of ['broken', 'locked']) {
    await expect(row(page, name)).toHaveAttribute('data-blocked', 'true')
    await expect(row(page, name).getByRole('button')).toHaveCount(0)
  }

  await row(page, 'locked').getByRole('link', { name: 'locked' }).click()
  await expect(page.getByRole('heading', { name: 'locked', level: 1 })).toBeVisible()
  await expect(page.getByText('Cannot read: Permission denied (os error 13)').first()).toBeVisible()
  await expect(page.getByRole('button', { name: /edit/i })).toHaveCount(0)
})

test('a project started by hand from a stack directory is "running elsewhere", not claimed', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await expect(row(page, 'web').locator('[data-note="elsewhere-webtest"]')).toHaveText('Running elsewhere as project webtest')
  // Still its own read-only foreign entry; the stack's status is the server's, unchanged.
  await expect(page.locator('[data-foreign="webtest"]')).toContainText("Started by hand from stack web's directory")
  await expect(row(page, 'web').locator('[data-status]')).toHaveText('Running')
})

test('a shadowed compose file is named beside the one in use (FR-004)', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web', { shadowed_files: ['docker-compose.yml'] })])
  await page.goto('/')
  const note = row(page, 'web').locator('[data-note="shadowed"]')
  await expect(note).toContainText('compose.yaml')
  await expect(note).toContainText('docker-compose.yml')
})

test('foreign projects and standalone containers are listed read-only, for admins only', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await expect(page.getByRole('heading', { name: /Not in the stacks directory/ })).toBeVisible()
  await expect(page.getByRole('heading', { name: /Standalone containers/ })).toBeVisible()
  for (const sel of ['[data-foreign="adhoc"]', '[data-standalone="loose-redis"]']) {
    const r = page.locator(sel)
    await expect(r).toContainText('Read-only')
    await expect(r.getByRole('button')).toHaveCount(0)
    await expect(r.getByRole('link')).toHaveCount(0)
  }
})

test('a user sees only their stacks and no foreign group', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.user(['web', 'db'])
  await page.goto('/')
  await expect(row(page, 'web')).toBeVisible()
  await expect(row(page, 'db')).toBeVisible()
  await expect(row(page, 'cache')).toHaveCount(0)
  await expect(page.locator('[data-foreign]')).toHaveCount(0)
  await expect(page.locator('[data-standalone]')).toHaveCount(0)
})

test('filters by name and by status', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await expect(row(page, 'web')).toBeVisible()

  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('CA')
  await expect(page.locator('[data-stack]')).toHaveCount(1)
  await expect(row(page, 'cache')).toBeVisible()

  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('')
  await page.getByRole('combobox', { name: 'Filter by status' }).selectOption('stopped')
  await expect(page.locator('[data-stack]')).toHaveCount(1)
  await expect(row(page, 'db')).toBeVisible()
  await expect(page.locator('[data-foreign], [data-standalone]')).toHaveCount(0)

  await page.getByRole('searchbox', { name: 'Filter by name' }).fill('zzz')
  await expect(page.getByText('No stacks match the filter.')).toBeVisible()
})

test('a live StacksChanged updates the list without a reload', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await expect(row(page, 'db').locator('[data-status]')).toHaveText('Stopped')
  await backend.waitForCmd('ListStacks')
  const next = backend.stacks.stacks.map((s) => (s.name === 'db' ? { ...s, status: 'running' as const, running_services: 1 } : s))
  backend.push({ type: 'StacksChanged', data: { ...backend.stacks, stacks: next } })
  await expect(row(page, 'db').locator('[data-status]')).toHaveText('Running')
  expect(backend.received.filter((c) => c.type === 'ListStacks')).toHaveLength(1)
})

test('the detail view shows containers, the compose file and a masked .env', async ({ page, backend }) => {
  const web = f.stack('web', {
    has_env: true,
    services: [
      {
        name: 'app',
        image: 'nginx:1.29',
        containers: [
          f.container({
            name: 'web-app-1',
            project: 'web',
            service: 'app',
            health: 'healthy',
            ports: [
              { container_port: 80, host_port: 8080, host_ip: '0.0.0.0', protocol: 'tcp' },
              { container_port: 80, host_port: 8080, host_ip: '::', protocol: 'tcp' },
            ],
          }),
        ],
      },
    ],
  })
  backend.stacks = f.stackList([web])
  backend.details.web = f.detail(web, {
    compose_text: 'services:\n  app:\n    image: nginx:1.29 # pinned\n',
    env: { kind: 'plain', text: 'PORT=8080\nAPI_TOKEN=abc123\n' },
  })
  await page.goto('/')
  await row(page, 'web').getByRole('link', { name: 'web' }).click()
  await expect(page).toHaveURL(/\/stacks\/web$/)

  const c = page.locator('[data-container="web-app-1"]')
  await expect(c.locator('[data-field="state"]')).toHaveText('running')
  await expect(c.locator('[data-field="health"]')).toHaveText('healthy')
  await expect(c.locator('[data-field="image"]')).toHaveText('nginx:1.29')
  await expect(c.locator('[data-field="uptime"]')).toHaveText('Up 3 hours')
  await expect(c.locator('[data-field="ports"]')).toHaveText('8080→80/tcp')

  await expect(page.getByRole('heading', { name: /Compose file compose\.yaml/ })).toBeVisible()
  const compose = page.locator('[data-code-view="yaml"]')
  await expect(compose).toContainText('image: nginx:1.29 # pinned')
  // Highlighted: CodeMirror wraps tokens in styled spans.
  await expect(compose.locator('.cm-line span').first()).toBeVisible()

  const env = page.locator('[data-code-view="env"]')
  await expect(env).toContainText('PORT=8080')
  await expect(env).toContainText('API_TOKEN=********')
  await expect(env).not.toContainText('abc123')
})

test('a stack the server refuses shows nothing about it', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web'), f.stack('secret')])
  backend.me = sessions.user(['web'])
  await page.goto('/stacks/secret')
  await expect(page.getByText('This stack is not available to you.')).toBeVisible()
  await expect(page.locator('[data-container]')).toHaveCount(0)
})

test('at 360 px wide there is no horizontal scroll', async ({ page, backend }) => {
  seed(backend)
  backend.stacks.stacks[0] = f.stack('a-very-long-stack-name-that-would-overflow-a-phone-screen-easily', {
    read_only_reason: 'the stack directory is a symlink to /mnt/some/very/long/path/that/keeps/going/and/going',
  })
  await page.setViewportSize({ width: 360, height: 740 })
  await page.goto('/')
  await expect(page.locator('[data-stack]').first()).toBeVisible()
  const overflow = () => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
  expect(await overflow()).toBeLessThanOrEqual(0)

  await page.goto('/stacks/cache')
  await expect(page.getByRole('heading', { name: 'cache', level: 1 })).toBeVisible()
  await expect(page.locator('[data-code-view="yaml"]')).toBeVisible()
  expect(await overflow()).toBeLessThanOrEqual(0)
})

test('wide screens keep the list beside the stack; narrow ones show one or the other', async ({ page, backend }) => {
  seed(backend)
  await page.setViewportSize({ width: 1366, height: 800 })
  await page.goto('/')
  await expect(page.locator('[data-layout="split"]')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Overview', level: 1 })).toBeVisible()

  await row(page, 'cache').getByRole('link', { name: 'cache' }).click()
  await expect(page.getByRole('heading', { name: 'cache', level: 1 })).toBeVisible()
  await expect(row(page, 'web')).toBeVisible()
  await expect(row(page, 'cache')).toHaveAttribute('data-selected', 'true')
  await expect(row(page, 'cache').getByRole('link', { name: 'cache' })).toHaveAttribute('aria-current', 'page')

  await page.setViewportSize({ width: 390, height: 844 })
  await expect(page.locator('[data-layout="split"]')).toHaveCount(0)
  await expect(row(page, 'web')).toHaveCount(0)
  await page.getByRole('link', { name: 'Stacks', exact: true }).last().click()
  await expect(row(page, 'web')).toBeVisible()
})

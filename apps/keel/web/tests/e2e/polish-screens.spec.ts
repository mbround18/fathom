/**
 * Spec 060 T055: every section, at a phone width (360 px) and a desktop width (1280 px), in the
 * dark and the light theme, fits the viewport: the page never scrolls sideways. Each section is
 * seeded with realistic, deliberately long content (repository URLs, commits, host names) so a
 * row or a header that cannot wrap shows up here. Against the fake backend (tests/support).
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../support/fake-backend'
import { governance, people } from '../support/fake-governance'
import { FakeHosts } from '../support/fake-hosts'
import { HEAD, NOW, PROJECT, WORKSPACE } from '../support/fake-repos'
import { FakeReviews } from '../support/fake-reviews'
import { FakeRuns, runInfo, stats } from '../support/fake-runs'
import { FakeSettings } from '../support/fake-settings'

const URL = 'https://git.example.com/platform-operations/infrastructure-ansible-playbooks.git'

/** An owner with an admin grant on `ops`, a recent passkey check, and something in every section. */
function seed(backend: FakeBackend): void {
  backend.me = fixtures.me({
    instance_role: 'owner',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true, enrolled: true, at: '2026-10-27T11:59:00Z' }),
  })

  governance(backend, (g) => {
    g.workspaces.push({ id: WORKSPACE, slug: 'ops', name: 'Operations', created: NOW, archived: false, projects: [], system: false })
    g.team('platform-operations', [people.ada(), people.grace()], [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'operator', profiles: ['base'] }])
  })

  const hosts = new FakeHosts(backend)
  hosts.connect('web-frontend-eu-west-1a.internal.example.com')
  hosts.connect('db-1')

  const runs = new FakeRuns(backend)
  runs.add(runInfo({ id: 'run-0001', job_name: 'database-maintenance', state: 'succeeded', ended_at: '2026-10-27T11:00:00Z' }))
  runs.add(runInfo({ id: 'run-0002', job_name: 'web', state: 'running' }), [
    {
      host: 'h1',
      name: 'web-frontend-eu-west-1a.internal.example.com',
      state: 'succeeded',
      refusal: null,
      started_at: NOW,
      ended_at: NOW,
      stats: stats({ ok: 12, changed: 3 }),
      first_failure: null,
    },
    { host: 'h2', name: 'web-2', state: 'running', refusal: null, started_at: NOW, ended_at: null, stats: null, first_failure: null },
  ])
  runs.log('run-0002', 'PLAY [web] *****\nTASK [nginx : render /etc/nginx/sites-available/a-very-long-virtual-host-name.conf] *****\n')

  new FakeReviews(backend)
  new FakeSettings(backend)

  backend.repos.bindings.push({
    id: 'binding-0001',
    project: PROJECT,
    workspace: WORKSPACE,
    url: URL,
    branch: 'main',
    transport: 'https',
    credential: null,
    known_host: null,
    poll_interval: 300,
    allow_force_push: false,
    signed_mode: true,
    override_quorum: 1,
    status: 'active',
    last_commit: HEAD,
    last_synced_at: NOW,
    last_sync_error: null,
    manifest_problems: [],
  })
  backend.repos.jobs.set('binding-0001', [
    { name: 'web', playbook: 'playbooks/web-frontend.yaml', profiles: ['base', 'web'], inventory: 'prod', blocked: null, defined_at_commit: HEAD },
  ])
  backend.repos.receivers.push({
    id: 'receiver-0001',
    workspace: WORKSPACE,
    name: 'forgejo-push-events',
    provider: 'forgejo',
    status: 'active',
    url: 'https://keel.example.com/hooks/receiver-0001/forgejo-push-events',
    previous_expires_at: null,
    bindings: ['binding-0001'],
    created_at: NOW,
    rotated_at: null,
  })
  backend.repos.deliver('receiver-0001', '3f2a9c1d-0b7e-6a5f-4c3b-2a1908f7e6d5')

  backend.inventories.seed('prod', ['web:\n  hosts:\n    web1:\n      http_port: 80\n'])
}

/**
 * The document's scroll and client widths, and the same for `<main>`: the shell scrolls `<main>`
 * on its own, so a section that is too wide scrolls there without widening the document.
 */
async function widths(page: Page): Promise<{ doc: [number, number]; main: [number, number]; offenders: string[] }> {
  return page.evaluate(() => {
    const root = document.documentElement
    const main = document.querySelector('main')
    // The innermost elements reaching past `<main>`'s right edge, so a failure names its cause.
    const edge = main ? main.getBoundingClientRect().right : root.clientWidth
    const offenders = [...(main ?? document.body).querySelectorAll('*')]
      .filter((el) => el.getBoundingClientRect().right > edge + 0.5)
      .filter((el) => ![...el.children].some((c) => c.getBoundingClientRect().right > edge + 0.5))
      .slice(0, 5)
      .map((el) => `<${el.tagName.toLowerCase()} class="${el.getAttribute('class') ?? ''}"> ${el.textContent.slice(0, 40)}`)
    return {
      doc: [root.scrollWidth, root.clientWidth],
      main: main ? [main.scrollWidth, main.clientWidth] : [0, 0],
      offenders,
    }
  })
}

interface Screen {
  name: string
  path: string
  /** What shows once the section has its data; the check runs after it. */
  ready: (page: Page) => Promise<void>
}

const h1 = (name: string | RegExp) => (page: Page) => expect(page.getByRole('heading', { name, level: 1 })).toBeVisible()

const SCREENS: Screen[] = [
  {
    name: 'runs',
    path: '/runs',
    ready: async (page) => {
      await expect(page.locator('[data-run-state="succeeded"]').first()).toBeVisible()
    },
  },
  {
    name: 'run',
    path: '/runs/run-0002',
    ready: async (page) => {
      await h1('Run of web')(page)
      await expect(page.getByRole('log', { name: 'Run output' })).toContainText('PLAY [web]')
    },
  },
  {
    name: 'reviews',
    path: '/reviews',
    ready: async (page) => {
      const row = page.locator('[data-review-state="open"]').first()
      await expect(row).toBeVisible()
      await row.getByRole('button', { name: /Open review of/ }).click()
      await expect(page.getByRole('region', { name: 'Commits' })).toBeVisible()
    },
  },
  {
    name: 'bindings',
    path: '/bindings',
    ready: async (page) => {
      await page.getByRole('button', { name: URL }).first().click()
      await expect(page.getByText('playbooks/web-frontend.yaml').first()).toBeVisible()
    },
  },
  {
    name: 'receivers',
    path: '/receivers',
    ready: async (page) => {
      await expect(page.getByText('forgejo-push-events').first()).toBeVisible()
    },
  },
  {
    name: 'inventory',
    path: '/inventory',
    ready: async (page) => {
      await page.getByRole('button', { name: 'prod' }).first().click()
      await expect(page.getByRole('region', { name: 'Inventory prod' })).toBeVisible()
    },
  },
  {
    name: 'hosts',
    path: '/hosts',
    ready: async (page) => {
      await page.getByRole('button', { name: 'Show web-frontend-eu-west-1a.internal.example.com' }).first().click()
      await expect(page.getByRole('region', { name: 'Host web-frontend-eu-west-1a.internal.example.com' })).toBeVisible()
    },
  },
  {
    name: 'teams',
    path: '/teams',
    ready: async (page) => {
      await expect(page.getByText('platform-operations').first()).toBeVisible()
    },
  },
  {
    name: 'users',
    path: '/users',
    ready: async (page) => {
      await expect(page.getByText('grace@example.com').first()).toBeVisible()
    },
  },
  { name: 'audit', path: '/audit', ready: h1('Audit') },
  { name: 'migration', path: '/migration', ready: h1('Migration') },
  { name: 'passkeys', path: '/account/passkeys', ready: h1('Passkeys') },
  { name: 'settings-enrollment', path: '/settings/enrollment', ready: h1('System Settings') },
  { name: 'settings-tokens', path: '/settings/tokens', ready: h1('System Settings') },
  { name: 'settings-identity', path: '/settings/identity', ready: h1('System Settings') },
  { name: 'settings-system', path: '/settings/system', ready: h1('System Settings') },
]

const WIDTHS = [360, 1280] as const
const THEMES = ['dark', 'light'] as const

for (const theme of THEMES) {
  for (const width of WIDTHS) {
    test.describe(`${theme} at ${String(width)} px`, () => {
      test.use({ viewport: { width, height: 800 } })

      for (const screen of SCREENS) {
        test(`${screen.name} fits the viewport`, async ({ page, backend }) => {
          seed(backend)
          await page.goto(screen.path)
          await page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark')
          await screen.ready(page)
          // Fonts and late layout settle before measuring.
          await page.evaluate(() => document.fonts.ready)
          const { doc, main, offenders } = await widths(page)
          expect(doc[0], `${screen.path}: the page is ${String(doc[0] - doc[1])} px wider than the viewport`).toBeLessThanOrEqual(doc[1])
          expect(main[0], `${screen.path}: the section scrolls ${String(main[0] - main[1])} px sideways: ${offenders.join('; ')}`).toBeLessThanOrEqual(main[1])
        })
      }
    })
  }
}

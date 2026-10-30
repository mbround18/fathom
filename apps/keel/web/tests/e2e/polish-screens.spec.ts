/**
 * Spec 060 T055: every section, at a phone width (360 px) and a desktop width (1280 px), in the
 * dark and the light theme, fits the viewport: the page never scrolls sideways. Each section is
 * seeded with realistic, deliberately long content (repository URLs, commits, host names) so a
 * row or a header that cannot wrap shows up here. Against the fake backend (tests/support).
 *
 * Spec 063 T051 adds the guided empty states, the grouped nav and an open glossary tip (SC-005,
 * FR-032): they fit too, and a nav group heading never takes the accent.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../support/fake-backend'
import { governance, people } from '../support/fake-governance'
import { FakeHosts } from '../support/fake-hosts'
import { HEAD, NOW, PROJECT, WORKSPACE } from '../support/fake-repos'
import { FakeReviews } from '../support/fake-reviews'
import { FakeRuns, runInfo, stats } from '../support/fake-runs'
import { FakeSettings } from '../support/fake-settings'
import { AVAILABLE_DIGEST, updateFixtures } from '../support/fake-updates'
import type { LastUpdateRun, UpdateStatus } from '../../src/protocol/wire'

const URL = 'https://git.example.com/platform-operations/infrastructure-ansible-playbooks.git'

/** An owner (or an admin, for the System tab's admin line) with an admin grant on `ops`, a recent passkey check, and something in every section. */
function seed(backend: FakeBackend, instance_role: 'owner' | 'admin' = 'owner'): void {
  backend.me = fixtures.me({
    instance_role,
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

/** Fails, naming the widest offenders, when the page or `<main>` scrolls sideways. */
async function expectFits(page: Page, label: string): Promise<void> {
  // Fonts and late layout settle before measuring.
  await page.evaluate(() => document.fonts.ready)
  const { doc, main, offenders } = await widths(page)
  expect(doc[0], `${label}: the page is ${String(doc[0] - doc[1])} px wider than the viewport`).toBeLessThanOrEqual(doc[1])
  expect(main[0], `${label}: the section scrolls ${String(main[0] - main[1])} px sideways: ${offenders.join('; ')}`).toBeLessThanOrEqual(main[1])
}

const setTheme = (page: Page, theme: 'dark' | 'light') =>
  page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark')

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
    // Opened, so the members, the grants and the grant editor are measured too.
    ready: async (page) => {
      await page.locator('[data-team="platform-operations"] > button').click()
      await expect(page.getByRole('list', { name: 'Grants of platform-operations' })).toBeVisible()
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

/** A watched image with a long, realistic reference, so the card's status line has to wrap. */
const LONG_IMAGE = 'registry.git.example.com/platform-operations/keel-server-builds:release-candidate'
const FAILED_RUN: LastUpdateRun = { run: 'run-update-0008', outcome: 'failed', digest: AVAILABLE_DIGEST, ended_at: '2026-10-28T11:00:00Z' }

interface SystemState {
  name: string
  role: 'owner' | 'admin'
  status: () => UpdateStatus
  /** What shows once the card (or the admin's line) has its state. */
  ready: (page: Page) => Promise<void>
}

const cardIn = (state: string, marker?: string) => async (page: Page) => {
  const card = page.locator(`[data-update-card="${state}"]`)
  await expect(card).toBeVisible()
  if (marker !== undefined) await expect(card.locator(marker)).toBeVisible()
}

/**
 * Spec 061 T084: Settings → System with the build panel and the update card in each state the
 * owner can see, and the admin's read-only line.
 */
const SYSTEM_STATES: SystemState[] = [
  { name: 'off', role: 'owner', status: () => updateFixtures.off(), ready: cardIn('off') },
  { name: 'not checked', role: 'owner', status: () => updateFixtures.notChecked(), ready: cardIn('not_checked') },
  { name: 'checking', role: 'owner', status: () => ({ ...updateFixtures.status('up_to_date'), state: 'checking' }), ready: cardIn('checking') },
  { name: 'up to date', role: 'owner', status: () => updateFixtures.status('up_to_date'), ready: cardIn('up_to_date') },
  { name: 'update available', role: 'owner', status: () => updateFixtures.status('update_available'), ready: cardIn('update_available', '[data-build="available"]') },
  {
    name: 'update by hand',
    role: 'owner',
    status: () => updateFixtures.status('update_available', { update_path: updateFixtures.manualPath('job_missing_param') }),
    ready: cardIn('update_available', '[data-update-manual="job_missing_param"]'),
  },
  { name: 'older build', role: 'owner', status: () => updateFixtures.status('older_build'), ready: cardIn('older_build') },
  { name: 'rebuilt', role: 'owner', status: () => updateFixtures.status('rebuilt'), ready: cardIn('rebuilt') },
  {
    name: 'cannot tell',
    role: 'owner',
    status: () => updateFixtures.status('cant_tell', { digest_compare: false, running: updateFixtures.running({ digest: null }) }),
    ready: cardIn('cant_tell', '[data-compared-by="revision"]'),
  },
  { name: 'failed check', role: 'owner', status: () => updateFixtures.failed('rate_limited'), ready: cardIn('up_to_date', '[data-update-error="rate_limited"]') },
  {
    name: 'update in progress',
    role: 'owner',
    status: () => updateFixtures.status('update_available', { update_run: 'run-update-0009' }),
    ready: cardIn('update_available', '[data-update-run="in_progress"]'),
  },
  {
    name: 'failed update',
    role: 'owner',
    status: () => updateFixtures.status('update_available', { last_update_run: FAILED_RUN }),
    ready: cardIn('update_available', '[data-update-run="failed"]'),
  },
  {
    name: 'admin line',
    role: 'admin',
    status: () => updateFixtures.status('update_available'),
    ready: async (page) => {
      await expect(page.locator('[data-update-line]')).toBeVisible()
    },
  },
]

/** A workspace admin on `ops` with every list in it empty, so each page shows its guided state. */
function seedEmpty(backend: FakeBackend): FakeHosts {
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  governance(backend, (g) => {
    g.workspaces.push({ id: WORKSPACE, slug: 'ops', name: 'Operations', created: NOW, archived: false, projects: [], system: false })
  })
  new FakeRuns(backend)
  new FakeReviews(backend).reviews = []
  return new FakeHosts(backend)
}

interface EmptyScreen {
  /** The `data-empty-page` values the screen shows. */
  pages: string[]
  path: string
  seed: (backend: FakeBackend) => void
}

/** Spec 063 FR-010's nine empty states (credentials shares Bindings, teams shares workspaces). */
const EMPTY_SCREENS: EmptyScreen[] = [
  { pages: ['runs'], path: '/runs', seed: seedEmpty },
  { pages: ['reviews'], path: '/reviews', seed: seedEmpty },
  { pages: ['bindings', 'credentials'], path: '/bindings', seed: seedEmpty },
  { pages: ['receivers'], path: '/receivers', seed: seedEmpty },
  { pages: ['inventory'], path: '/inventory', seed: seedEmpty },
  { pages: ['hosts'], path: '/hosts', seed: seedEmpty },
  {
    pages: ['workspaces', 'teams'],
    path: '/teams',
    seed: (backend) => {
      backend.me = fixtures.me({ instance_role: 'owner' })
      governance(backend)
    },
  },
  {
    pages: ['workspace-picker'],
    path: '/runs',
    seed: (backend) => {
      backend.me = fixtures.me({ instance_role: 'owner' })
    },
  },
]

const NAV_GROUPS = ['pipelines', 'infrastructure', 'administration'] as const
// A glossary term the guided Hosts state shows (src/help/glossary.ts; the e2e build can't import it).
const OPEN_TIP = 'base-profile'

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
          await setTheme(page, theme)
          await screen.ready(page)
          await page.evaluate(() => document.fonts.ready)
          // KEEL_SHOTS=<dir> also saves each screen, for comparing a visual pass before and after.
          if (process.env.KEEL_SHOTS) await page.screenshot({ path: `${process.env.KEEL_SHOTS}/${screen.name}-${theme}-${String(width)}.png`, fullPage: true })
          await expectFits(page, screen.path)
        })
      }

      for (const state of SYSTEM_STATES) {
        test(`settings-system (${state.name}) fits the viewport`, async ({ page, backend }) => {
          seed(backend, state.role)
          backend.updates.status = { ...state.status(), image: LONG_IMAGE }
          await page.goto('/settings/system')
          await setTheme(page, theme)
          await expect(page.locator('[data-build-panel]')).toBeVisible()
          await state.ready(page)
          await expectFits(page, state.name)
        })
      }

      for (const screen of EMPTY_SCREENS) {
        test(`empty ${screen.pages.join(' and ')} fits the viewport`, async ({ page, backend }) => {
          screen.seed(backend)
          await page.goto(screen.path)
          await setTheme(page, theme)
          for (const name of screen.pages) await expect(page.locator(`[data-empty-page="${name}"]`)).toBeVisible()
          if (process.env.KEEL_SHOTS) await page.screenshot({ path: `${process.env.KEEL_SHOTS}/empty-${screen.pages[0]}-${theme}-${String(width)}.png`, fullPage: true })
          await expectFits(page, `empty ${screen.path}`)
        })
      }

      test('an open glossary tip fits the viewport', async ({ page, backend }) => {
        seedEmpty(backend)
        await page.goto('/hosts')
        await setTheme(page, theme)
        await page.locator(`[data-infotip="${OPEN_TIP}"]`).first().click()
        await expect(page.locator(`[data-infotip-content="${OPEN_TIP}"]`)).toBeVisible()
        await expectFits(page, `the ${OPEN_TIP} tip`)
        // The popover is portalled outside <main>, so check it against the viewport too.
        const box = await page.locator(`[data-infotip-content="${OPEN_TIP}"]`).boundingBox()
        expect(box?.x ?? -1).toBeGreaterThanOrEqual(0)
        expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(width)
      })

      test('the grouped nav fits, and no group heading takes the accent', async ({ page, backend }) => {
        seed(backend)
        await page.goto('/runs')
        await setTheme(page, theme)
        const nav = page.getByRole('navigation', { name: 'Sections' })
        const runs = nav.getByRole('link', { name: 'Runs' })
        await expect(runs).toHaveAttribute('aria-current', 'page')
        const current = await runs.evaluate((el) => getComputedStyle(el).color)
        for (const id of NAV_GROUPS) {
          const heading = page.locator(`[data-nav-group="${id}"] > span`)
          expect(await heading.evaluate((el) => getComputedStyle(el).color), `${id} heading`).not.toBe(current)
        }
        await expectFits(page, 'the grouped nav')
      })
    })
  }
}

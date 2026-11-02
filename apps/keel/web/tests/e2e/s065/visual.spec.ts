/**
 * Spec 065 T006 (SC-003, R12): screenshot goldens of the screens built from the pieces that move
 * into `@companion/ui/kit`, taken before the move. After it, every shot must match pixel for pixel:
 * the move changes where the code lives, never what Keel looks like. Each scene at 360 and 1280 px,
 * dark and light, against the fake backend with the clock fixed.
 *
 * Run: `KEEL_GOLDENS=1 pnpm --filter @companion/keel-web e2e --project s065`. Not in CI's default
 * run: goldens are machine-specific, and these were made on the dev box.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance, people } from '../../support/fake-governance'
import { FakeHosts } from '../../support/fake-hosts'
import { NOW, WORKSPACE } from '../../support/fake-repos'
import { FakeReviews } from '../../support/fake-reviews'
import { FakeRuns, runInfo, stats } from '../../support/fake-runs'
import { FakeSettings } from '../../support/fake-settings'

/** An owner with an admin grant on `ops`; `runs` decides whether the run list has rows. */
function seed(backend: FakeBackend, runs: 'rows' | 'empty' = 'rows'): void {
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
  hosts.connect('db-1')
  const fake = new FakeRuns(backend)
  if (runs === 'rows') {
    fake.add(runInfo({ id: 'run-0001', job_name: 'database-maintenance', state: 'succeeded', ended_at: '2026-10-27T11:00:00Z' }))
    fake.add(runInfo({ id: 'run-0002', job_name: 'web', state: 'running' }), [
      { host: 'h1', name: 'web-1', state: 'succeeded', refusal: null, started_at: NOW, ended_at: NOW, stats: stats({ ok: 12, changed: 3 }), first_failure: null },
      { host: 'h2', name: 'web-2', state: 'running', refusal: null, started_at: NOW, ended_at: null, stats: null, first_failure: null },
    ])
    fake.log('run-0002', 'PLAY [web] *****\n')
  }
  new FakeReviews(backend)
  new FakeSettings(backend)
}

interface Scene {
  name: string
  path: string
  runs?: 'rows' | 'empty'
  /** Extra set-up on the backend before the page opens. */
  before?: (backend: FakeBackend) => void
  /** Brings the page to the state the shot shows. */
  ready: (page: Page) => Promise<void>
}

const h1 = (page: Page, name: string) => expect(page.getByRole('heading', { name, level: 1 })).toBeVisible()

const SCENES: readonly Scene[] = [
  {
    name: 'runs-list',
    path: '/runs',
    ready: async (page) => {
      await expect(page.locator('[data-run-state="succeeded"]:visible')).toHaveCount(1)
      await expect(page.locator('[data-run-state="running"]:visible')).toHaveCount(1)
    },
  },
  {
    name: 'runs-empty',
    path: '/runs',
    runs: 'empty',
    ready: async (page) => {
      await expect(page.locator('[data-empty-page="runs"]')).toBeVisible()
    },
  },
  {
    name: 'run-cancel-confirm',
    path: '/runs/run-0002',
    ready: async (page) => {
      await h1(page, 'Run of web')
      await page.getByRole('button', { name: 'Cancel run' }).click()
      await expect(page.getByRole('dialog').getByRole('list', { name: 'Affected' })).toContainText('web-2')
    },
  },
  {
    name: 'hosts-failed',
    path: '/hosts',
    before: (backend) => {
      backend.chain('ListHosts', () => [{ type: 'OpError', data: { code: 'internal', message: 'The hosts store is down.' } }])
    },
    ready: async (page) => {
      await expect(page.locator('[data-pane-state="error"]')).toContainText('Failed to load hosts.')
    },
  },
  {
    name: 'teams-open',
    path: '/teams',
    ready: async (page) => {
      await page.locator('[data-team="platform-operations"] > button').click()
      await expect(page.getByRole('list', { name: 'Grants of platform-operations' })).toBeVisible()
    },
  },
  {
    name: 'help-guide',
    path: '/help/getting-started',
    ready: async (page) => {
      await expect(page.locator('[data-help-guide="getting-started"]')).toBeVisible()
    },
  },
  {
    name: 'reviews-term-tip',
    path: '/reviews',
    ready: async (page) => {
      await page.locator('[data-infotip="trusted-signers"]').first().hover()
      await expect(page.locator('[data-infotip-content="trusted-signers"]')).toBeVisible()
    },
  },
  {
    name: 'settings-tabs',
    path: '/settings/system',
    ready: async (page) => {
      await h1(page, 'System Settings')
      await expect(page.locator('[data-build-panel]')).toBeVisible()
    },
  },
]

const WIDTHS = [360, 1280] as const
const THEMES = ['dark', 'light'] as const

for (const theme of THEMES) {
  for (const width of WIDTHS) {
    test.describe(`${theme} at ${String(width)} px`, () => {
      test.use({ viewport: { width, height: 800 } })

      for (const scene of SCENES) {
        test(scene.name, async ({ page, backend }) => {
          seed(backend, scene.runs)
          scene.before?.(backend)
          await page.clock.setFixedTime(new Date(NOW))
          await page.goto(scene.path)
          await page.evaluate((dark) => document.documentElement.classList.toggle('dark', dark), theme === 'dark')
          await scene.ready(page)
          await page.evaluate(() => document.fonts.ready)
          await expect(page).toHaveScreenshot(`${scene.name}-${theme}-${String(width)}.png`, {
            fullPage: true,
            animations: 'disabled',
            maxDiffPixelRatio: 0,
          })
        })
      }
    })
  }
}

/**
 * Spec 063 T014 (US1, FR-010 to FR-016): every main list that is empty says why, what to do next
 * and where to read more. Someone who can act gets one primary action; someone who can't is told
 * who can, with no button. A filter that empties a list offers to clear it instead, and loading
 * never reads as empty. Against the fake backend with empty lists.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance } from '../../support/fake-governance'
import { FakeHosts } from '../../support/fake-hosts'
import { WORKSPACE } from '../../support/fake-repos'
import { FakeReviews } from '../../support/fake-reviews'
import { FakeRuns } from '../../support/fake-runs'
import type { TeamRole } from '../../../src/protocol/generated/TeamRole'

/** A person granted `role` on the one workspace, with every list in it empty. */
function granted(backend: FakeBackend, role: TeamRole): FakeHosts {
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role, profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  governance(backend, (g) => {
    g.workspaces.push({ id: WORKSPACE, slug: 'ops', name: 'Ops', created: '2026-10-27T12:00:00Z', archived: false, projects: [], system: false })
  })
  new FakeRuns(backend)
  new FakeReviews(backend).reviews = []
  return new FakeHosts(backend)
}

/** Each workspace page's main list: where it lives and the guide its empty state links to. */
const PAGES = [
  { path: '/hosts', page: 'hosts', guide: '/help/hosts#enroll-a-host' },
  { path: '/receivers', page: 'receivers', guide: '/help/receivers#create-a-receiver' },
  { path: '/bindings', page: 'bindings', guide: '/help/bindings#bind-a-repository' },
  { path: '/bindings', page: 'credentials', guide: '/help/bindings#credentials' },
  { path: '/runs', page: 'runs', guide: '/help/getting-started#your-first-run' },
  { path: '/reviews', page: 'reviews', guide: '/help/reviews#trusted-signers' },
  { path: '/inventory', page: 'inventory', guide: '/help/inventory' },
] as const

async function expectGuided(page: Page, name: string, guide: string): Promise<void> {
  const empty = page.locator(`[data-empty-page="${name}"]`)
  await expect(empty).toHaveAttribute('data-empty', 'guided')
  await expect(empty.locator('[data-empty-action]').getByRole('button').or(empty.locator('[data-empty-action] a'))).toHaveCount(1)
  const steps = await empty.locator('ol > li').count()
  expect(steps).toBeGreaterThanOrEqual(2)
  expect(steps).toBeLessThanOrEqual(3)
  await expect(empty.locator('[data-guide-link]')).toHaveAttribute('href', guide)
  await expect(empty.locator('[data-who-can]')).toHaveCount(0)
}

async function expectRestricted(page: Page, name: string): Promise<void> {
  const empty = page.locator(`[data-empty-page="${name}"]`)
  await expect(empty).toHaveAttribute('data-empty', 'restricted')
  await expect(empty.locator('[data-who-can]')).toBeVisible()
  // No action to take; a glossary info icon in a step is not one.
  await expect(empty.locator('button:not([data-infotip])')).toHaveCount(0)
  await expect(empty.locator('[data-empty-action]')).toHaveCount(0)
}

test.describe('an operator', () => {
  test.beforeEach(({ backend }) => {
    granted(backend, 'operator')
  })

  for (const { path, page: name, guide } of PAGES) {
    test(`${name}: guided, with one action, steps and a guide`, async ({ page }) => {
      await page.goto(path)
      await expectGuided(page, name, guide)
    })
  }

  test('the Hosts action opens the Enroll dialog', async ({ page }) => {
    await page.goto('/hosts')
    await page.locator('[data-empty-action]').getByRole('button', { name: 'Enroll a host' }).click()
    await expect(page.getByRole('dialog').getByRole('form', { name: 'Enroll a host' })).toBeVisible()
  })

  test('the Bindings action opens the New binding form', async ({ page }) => {
    await page.goto('/bindings')
    await page.locator('[data-empty-page="bindings"] [data-empty-action]').getByRole('button').click()
    await expect(page.getByRole('form', { name: 'New binding' })).toBeVisible()
    await expect(page.locator('[data-empty-page="bindings"]')).toHaveCount(0)
  })

  test('the Runs action goes to Bindings', async ({ page }) => {
    await page.goto('/runs')
    await page.locator('[data-empty-action]').getByRole('link', { name: 'Go to bindings' }).click()
    await expect(page).toHaveURL(/\/bindings$/)
  })

  test('the Receivers action puts the cursor in the new receiver name', async ({ page }) => {
    await page.goto('/receivers')
    await page.locator('[data-empty-action]').getByRole('button', { name: 'Create a receiver' }).click()
    await expect(page.getByRole('form', { name: 'New receiver' }).getByLabel('Name')).toBeFocused()
  })

  test('secondary lists keep their one-line message', async ({ page }) => {
    await page.goto('/receivers')
    await expect(page.getByRole('table', { name: 'Deliveries' })).toContainText('No deliveries yet.')
    await expect(page.locator('[data-empty-page="receivers"]')).toBeVisible()
  })
})

test.describe('a viewer', () => {
  test.beforeEach(({ backend }) => {
    granted(backend, 'viewer')
  })

  for (const { path, page: name } of PAGES) {
    test(`${name}: restricted, says who can, no button`, async ({ page }) => {
      await page.goto(path)
      await expectRestricted(page, name)
    })
  }
})

test.describe('teams', () => {
  test('an owner gets Create a workspace and Create a team', async ({ page, backend }) => {
    backend.me = fixtures.me({ instance_role: 'owner' })
    governance(backend)
    await page.goto('/teams')
    await expectGuided(page, 'workspaces', '/help/getting-started#create-a-workspace')
    await expectGuided(page, 'teams', '/help/getting-started#give-people-access')
    await page.locator('[data-empty-page="teams"] [data-empty-action]').getByRole('button').click()
    await expect(page.getByRole('form', { name: 'New team' })).toBeVisible()
  })

  test('a plain user is told who can', async ({ page, backend }) => {
    backend.me = fixtures.me({ instance_role: 'user' })
    governance(backend)
    await page.goto('/teams')
    await expectRestricted(page, 'workspaces')
    await expectRestricted(page, 'teams')
  })
})

test.describe('no workspace', () => {
  test('an owner is sent to Teams to create one', async ({ page, backend }) => {
    backend.me = fixtures.me({ instance_role: 'owner' })
    await page.goto('/runs')
    const empty = page.locator('[data-empty-page="workspace-picker"]')
    await expect(empty).toHaveAttribute('data-empty', 'guided')
    await expect(empty.locator('[data-guide-link]')).toHaveAttribute('href', '/help/getting-started#create-a-workspace')
    await expect(empty.locator('[data-guide-link]')).toHaveAccessibleName('Learn more: Create a workspace')
    await empty.getByRole('link', { name: 'Create a workspace', exact: true }).click()
    await expect(page).toHaveURL(/\/teams$/)
  })

  test('a plain user is told an owner or admin grants workspaces', async ({ page, backend }) => {
    backend.me = fixtures.me({ instance_role: 'user' })
    await page.goto('/bindings')
    const empty = page.locator('[data-empty-page="workspace-picker"]')
    await expectRestricted(page, 'workspace-picker')
    await expect(empty.locator('[data-who-can]')).toContainText('An owner or admin can')
  })
})

test('a filter that empties Hosts offers to clear it', async ({ page, backend }) => {
  granted(backend, 'operator').connect('web-1')
  await page.goto('/hosts')
  const table = page.getByRole('table', { name: 'Hosts' })
  await expect(table).toContainText('web-1')

  await page.getByLabel('Only hosts changed outside Keel').check()
  const none = page.locator('[data-empty="no-match"]')
  await expect(none).toContainText('No hosts match.')
  await expect(page.locator('[data-empty-page="hosts"]')).toHaveCount(0)

  await none.getByRole('button', { name: 'Clear filter' }).click()
  await expect(table).toContainText('web-1')
  await expect(page.getByLabel('Only hosts changed outside Keel')).not.toBeChecked()
})

test('a review filter that empties the queue offers to clear it', async ({ page, backend }) => {
  granted(backend, 'operator')
  await page.goto('/reviews')
  await expect(page.locator('[data-empty-page="reviews"]')).toHaveAttribute('data-empty', 'guided')
  await page.getByLabel('Show').selectOption('denied')
  const none = page.locator('[data-empty="no-match"]')
  await expect(none).toContainText('No reviews match.')
  await none.getByRole('button', { name: 'Clear filter' }).click()
  await expect(page.getByLabel('Show')).toHaveValue('open')
  await expect(page.locator('[data-empty-page="reviews"]')).toHaveAttribute('data-empty', 'guided')
})

test('while a list loads, or after it fails, no empty state shows', async ({ page, backend }) => {
  granted(backend, 'operator')
  // Hosts never answers; Runs fails.
  backend.on('ListHosts', () => [])
  backend.on('ListRuns', () => [{ type: 'OpError', data: { code: 'internal', message: 'The run list failed.' } }])

  await page.goto('/hosts')
  await expect(page.getByRole('heading', { name: 'Hosts', level: 1 })).toBeVisible()
  await expect(page.getByRole('status').filter({ hasText: 'Loading hosts' })).toBeVisible()
  await expect(page.locator('[data-empty]')).toHaveCount(0)

  await page.goto('/runs')
  await expect(page.getByRole('alert')).toContainText('The run list failed.')
  await expect(page.locator('[data-empty]')).toHaveCount(0)
})

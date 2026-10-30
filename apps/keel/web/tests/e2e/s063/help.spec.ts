/**
 * Spec 063 T012 (R6): the Keel guides, bundled into the app at `/help`. The index lists them, each
 * new guide renders with its title, a `#anchor` scrolls to its heading, links between guides stay
 * in the tab, and an unknown guide is Not found. Against the fake backend, signed in as its
 * default admin; nothing here needs data.
 */
import { expect, test } from '../../support/fake-backend'

const ALL_GUIDES = [
  'getting-started',
  'hosts',
  'receivers',
  'bindings',
  'reviews',
  'inventory',
  'enrollment-template',
  'oidc-roles',
  'system-workspace',
  'updates',
] as const

const NEW_GUIDES: Record<string, string> = {
  'getting-started': 'Getting started',
  hosts: 'Hosts and enrolment',
  receivers: 'Receivers',
  bindings: 'Bindings and credentials',
  reviews: 'Reviews and trusted signers',
  inventory: 'Inventory',
}

test('/help lists every guide', async ({ page }) => {
  await page.goto('/help')
  const index = page.locator('[data-help-guide="README"]')
  await expect(index).toBeVisible()
  await expect(page.getByRole('heading', { level: 1, name: 'Keel user guides' })).toBeVisible()
  for (const slug of ALL_GUIDES) await expect(index.locator(`a[href="/help/${slug}"]`).first()).toBeVisible()
})

for (const [slug, title] of Object.entries(NEW_GUIDES)) {
  test(`${slug} renders with its title`, async ({ page }) => {
    await page.goto(`/help/${slug}`)
    await expect(page.locator(`[data-help-guide="${slug}"]`)).toBeVisible()
    await expect(page.getByRole('heading', { level: 1, name: title, exact: true })).toBeVisible()
    // The `#` title is the page's heading, not repeated in the article.
    await expect(page.locator(`[data-help-guide="${slug}"] h1`)).toHaveCount(0)
    await expect(page.locator(`[data-help-guide="${slug}"] h2[id]`).first()).toBeVisible()
  })
}

test('a link between guides stays in the tab', async ({ page }) => {
  await page.goto('/help')
  await page.locator('[data-help-guide="README"]').getByRole('link', { name: 'Getting started' }).first().click()
  await expect(page).toHaveURL(/\/help\/getting-started$/)
  await expect(page.locator('[data-help-guide="getting-started"]')).toBeVisible()
  await expect(page.locator('#create-a-workspace')).toBeVisible()
})

test('/help/reviews#trusted-signers scrolls its heading into view', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 320 })
  await page.goto('/help/reviews#trusted-signers')
  await expect(page.locator('[data-help-guide="reviews"]')).toBeVisible()
  await expect(page.locator('#trusted-signers')).toBeInViewport()
})

test('an anchor far down a long guide is scrolled to', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 480 })
  // Without the anchor the heading is below the fold, so the check after it means something.
  await page.goto('/help/updates')
  await expect(page.locator('[data-help-guide="updates"]')).toBeVisible()
  await expect(page.locator('#rolling-back')).not.toBeInViewport()

  await page.goto('/help/updates#rolling-back')
  await expect(page.locator('#rolling-back')).toBeInViewport()
})

test('an unknown guide is Not found', async ({ page }) => {
  await page.goto('/help/no-such-guide')
  await expect(page.locator('[data-page="not-found"]')).toBeVisible()
  await expect(page.locator('[data-help-guide]')).toHaveCount(0)
})

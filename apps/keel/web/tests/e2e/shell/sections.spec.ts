/** The section navigation and its empty pages (spec 055 T030). */
import { expect, test } from '../../support/fake-backend'

const SECTIONS = ['Runs', 'Reviews', 'Bindings', 'Receivers', 'Inventory', 'Hosts', 'Teams', 'Audit', 'Migration']

test('/ goes to runs', async ({ page }) => {
  await page.goto('/')
  await expect(page).toHaveURL(/\/runs$/)
  await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible()
})

test('every section opens from the navigation', async ({ page }) => {
  await page.goto('/runs')
  const nav = page.getByRole('navigation', { name: 'Sections' })
  for (const name of SECTIONS) {
    await nav.getByRole('link', { name }).click()
    await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible()
    await expect(nav.getByRole('link', { name })).toHaveAttribute('aria-current', 'page')
    await expect(page).toHaveURL(new RegExp(`/${name.toLowerCase()}$`))
  }
})

test('an unknown path is not found', async ({ page }) => {
  await page.goto('/nowhere')
  await expect(page.getByRole('heading', { name: 'Not found' })).toBeVisible()
  await page.getByRole('link', { name: 'Back to runs' }).click()
  await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible()
})

test('narrow screens keep the navigation in one scrolling strip', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 700 })
  await page.goto('/runs')
  const nav = page.getByRole('navigation', { name: 'Sections' })
  await nav.getByRole('link', { name: 'Migration' }).click()
  await expect(page.getByRole('heading', { name: 'Migration' })).toBeVisible()
  const width = await page.evaluate(() => document.documentElement.scrollWidth)
  expect(width).toBeLessThanOrEqual(375)
})

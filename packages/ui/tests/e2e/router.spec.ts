/** The router and Link: a plain click navigates in-app; a modified click is left to the browser. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'router'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test('links: a plain click navigates in-app, the path matches, and Back undoes it', async ({ page }) => {
  await openPiece(page, { piece, state: 'links' })
  const path = page.locator('[data-path]')
  await expect(path).toHaveText('/')
  await expect(page.locator('[data-widget]')).toHaveText('No widget open.')
  await expect(page.getByRole('link', { name: 'Blue widget' })).toHaveAttribute('href', '/widgets/blue')

  await page.getByRole('link', { name: 'Blue widget' }).click()
  await expect(path).toHaveText('/widgets/blue')
  await expect(page.locator('[data-widget]')).toHaveText('Showing blue widget.')
  await expect(page).toHaveURL(/\/widgets\/blue$/)
  // Same document: the fixture is still there, so nothing reloaded.
  await expect(page.getByRole('link', { name: 'All widgets' })).toBeVisible()

  await page.goBack()
  await expect(path).toHaveText('/')
})

test('links: Enter on a focused link navigates', async ({ page }) => {
  await openPiece(page, { piece, state: 'links' })
  await page.keyboard.press('Tab')
  await expect(page.getByRole('link', { name: 'All widgets' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.locator('[data-path]')).toHaveText('/widgets')
})

test('links: a ctrl/cmd or shift click is left to the browser, and the page stays put', async ({ page, context }) => {
  context.on('page', (opened) => {
    void opened.close()
  })
  await openPiece(page, { piece, state: 'links' })
  const link = page.getByRole('link', { name: 'Blue widget' })
  await link.click({ modifiers: ['ControlOrMeta'] })
  await link.click({ modifiers: ['Shift'] })
  await expect(page.locator('[data-path]')).toHaveText('/')
  await expect(page).toHaveURL(/\?piece=router/)
})

test('guarded: unsaved work holds a click and Back; once saved, the click goes', async ({ page }) => {
  await openPiece(page, { piece, state: 'guarded' })
  const path = page.locator('[data-path]')
  const dirty = page.getByRole('checkbox', { name: 'Unsaved changes' })

  await dirty.check()
  await page.getByRole('link', { name: 'Blue widget' }).click()
  await expect(path).toHaveText('/')

  await dirty.uncheck()
  await page.getByRole('link', { name: 'Blue widget' }).click()
  await expect(path).toHaveText('/widgets/blue')

  await dirty.check()
  await page.goBack()
  await expect(page).toHaveURL(/\/widgets\/blue$/)
  await expect(path).toHaveText('/widgets/blue')
})

test('links: at 360 px a tap navigates', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'links' })
  await page.getByRole('link', { name: 'All widgets' }).tap()
  await expect(page.locator('[data-path]')).toHaveText('/widgets')
})

for (const state of ['links', 'guarded']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

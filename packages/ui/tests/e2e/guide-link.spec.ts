/** GuideLink: "Learn more" to a guide's route in the same tab; nothing when the app has no guides. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'guide-link'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test('default: Learn more, marked with its guide, goes to the route from the keyboard', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  const link = page.getByRole('link', { name: 'Learn more' })
  await expect(link).toHaveAttribute('data-guide-link', 'widgets')
  await expect(link).toHaveAttribute('href', '/help/widgets#sharing-a-widget')
  await expect(link).not.toHaveAttribute('target')
  await page.keyboard.press('Tab')
  await expect(link).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/\/help\/widgets#sharing-a-widget$/)
})

test('label: the caller’s words name the link', async ({ page }) => {
  await openPiece(page, { piece, state: 'label' })
  const link = page.getByRole('link', { name: 'Read the widgets guide' })
  await expect(link).toHaveAttribute('href', '/help/widgets')
  await expect(link).toHaveAttribute('data-guide-link', 'widgets')
})

test('no-guides: without guideHref there is no link, only the sentence', async ({ page }) => {
  await openPiece(page, { piece, state: 'no-guides' })
  await expect(page.getByText('Share a widget with the people who need it.')).toBeVisible()
  await expect(page.getByRole('link')).toHaveCount(0)
})

test('at 360 px a tap follows the link', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'default' })
  await page.getByRole('link', { name: 'Learn more' }).tap()
  await expect(page).toHaveURL(/\/help\/widgets#sharing-a-widget$/)
})

for (const state of ['default', 'label', 'no-guides']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

/** Markdown: guides get anchored headings and in-app guide links; everything stays safe. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'markdown'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test('guide: h2 and h3 get ids from their text; the h1 does not', async ({ page }) => {
  await openPiece(page, { piece, state: 'guide' })
  const root = page.locator('[data-markdown]')
  await expect(root.getByRole('heading', { level: 1, name: 'Widgets' })).not.toHaveAttribute('id')
  await expect(root.getByRole('heading', { level: 2, name: 'Sharing a widget' })).toHaveAttribute('id', 'sharing-a-widget')
  await expect(root.getByRole('heading', { level: 3, name: 'Who can see it' })).toHaveAttribute('id', 'who-can-see-it')
  await expect(root.getByRole('heading', { level: 2, name: 'Removing a widget' })).toHaveAttribute('id', 'removing-a-widget')
})

test('guide: from the keyboard, the anchor stays on the page and the guide link goes to its route', async ({ page }) => {
  await openPiece(page, { piece, state: 'guide' })
  const anchor = page.getByRole('link', { name: 'sharing one' })
  await expect(anchor).toHaveAttribute('href', '#sharing-a-widget')
  await expect(anchor).not.toHaveAttribute('target')
  const guide = page.getByRole('link', { name: 'first steps' })
  await expect(guide).toHaveAttribute('href', '/help/example#first-steps')
  await expect(guide).not.toHaveAttribute('target')

  await page.keyboard.press('Tab')
  await expect(anchor).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/#sharing-a-widget$/)
  // Following the anchor moves the browser's Tab start to the heading, past the guide link.
  await guide.focus()
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(/\/help\/example#first-steps$/)
})

test('guide: an img/ image the app does not bundle is never fetched, and the table is a table', async ({ page }) => {
  await openPiece(page, { piece, state: 'guide' })
  const root = page.locator('[data-markdown]')
  await expect(root.locator('img')).toHaveCount(0)
  await expect(root).toContainText('[the share menu]')
  const table = root.getByRole('table')
  await expect(table.getByRole('columnheader')).toHaveText(['Role', 'Can see it', 'Can change it'])
  await expect(table.getByRole('row')).toHaveCount(4)
})

test('guide: at 360 px the table scrolls inside itself and a tap on the guide link goes there', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'guide' })
  const table = page.getByRole('table')
  const [scroll, client] = await table.evaluate((el) => [el.scrollWidth, el.clientWidth])
  expect(scroll).toBeGreaterThanOrEqual(client)
  await page.getByRole('link', { name: 'first steps' }).tap()
  await expect(page).toHaveURL(/\/help\/example#first-steps$/)
})

test('instructions: raw HTML shows as text, links open a new tab, images stay links, javascript: is dropped', async ({ page }) => {
  await openPiece(page, { piece, state: 'instructions' })
  const root = page.locator('[data-markdown]')
  await expect(root.locator('script, b')).toHaveCount(0)
  await expect(root).toContainText('<script>alert("hello")</script>')
  await expect(root).toContainText('The <b>full</b> list')
  await expect(root.getByRole('heading', { level: 2, name: 'Before you start' })).not.toHaveAttribute('id')

  const notes = root.getByRole('link', { name: 'network notes' })
  await expect(notes).toHaveAttribute('target', '_blank')
  await expect(notes).toHaveAttribute('rel', 'noopener noreferrer')

  const image = root.getByRole('link', { name: '[the network diagram]' })
  await expect(image).toHaveAttribute('data-image-link', 'true')
  await expect(image).toHaveAttribute('href', 'https://widgets.example.test/img/network.png')
  await expect(root.locator('img')).toHaveCount(0)

  await expect(root.getByText('Do not follow this')).toBeVisible()
  await expect(root.getByRole('link', { name: 'Do not follow this' })).toHaveCount(0)
})

for (const state of ['guide', 'instructions']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

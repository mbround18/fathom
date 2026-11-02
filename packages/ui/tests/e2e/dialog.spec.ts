/** Dialog: opens from its trigger, traps focus, closes on Escape and gives focus back. */
import { expect, test, type Page } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'
import { settled } from '../support/settle'

const piece = 'dialog'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

async function open(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Rename widget' }).click()
  await expect(page.getByRole('dialog', { name: 'Rename widget' })).toBeVisible()
  await settled(page)
}

test('opens from the keyboard, with its name, description and marker', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Rename widget' })).toBeFocused()
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog', { name: 'Rename widget' })
  await expect(dialog).toHaveAttribute('data-rename-dialog', 'true')
  await expect(dialog).toHaveAccessibleDescription('The new name shows everywhere the widget is listed.')
  await expect(dialog.locator('[data-summary]')).toHaveText('The new name shows everywhere the widget is listed.')
  // Focus starts inside, on the first field.
  await expect(dialog.getByRole('textbox', { name: 'Name' })).toBeFocused()
})

test('traps focus: Tab and Shift+Tab stay inside the dialog', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  await open(page)
  const dialog = page.getByRole('dialog')
  for (let i = 0; i < 6; i++) {
    await page.keyboard.press('Tab')
    expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true)
  }
  for (let i = 0; i < 6; i++) {
    await page.keyboard.press('Shift+Tab')
    expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true)
  }
})

test('Escape closes it and focus returns to the trigger, with nothing saved', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  await page.getByRole('button', { name: 'Rename widget' }).focus()
  await page.keyboard.press('Enter')
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.type('Green widget')
  await page.keyboard.press('Escape')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Rename widget' })).toBeFocused()
  await expect(page.locator('[data-saved]')).toHaveText('Name: Blue widget')
})

test('Save from the keyboard closes it, and the close button closes it too', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  await open(page)
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.type('Green widget')
  await page.getByRole('button', { name: 'Save' }).focus()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.locator('[data-saved]')).toHaveText('Name: Green widget')
  await expect(page.getByRole('button', { name: 'Rename widget' })).toBeFocused()

  await open(page)
  await page.getByRole('button', { name: 'Close' }).focus()
  await page.keyboard.press('Space')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Rename widget' })).toBeFocused()
})

test('without a description, it is still described by its title', async ({ page }) => {
  await openPiece(page, { piece, state: 'no-description' })
  await open(page)
  const dialog = page.getByRole('dialog', { name: 'Rename widget' })
  await expect(dialog.locator('[data-summary]')).toHaveCount(0)
  await expect(dialog).toHaveAccessibleDescription('Rename widget')
})

test('at 360 px it fits the screen less a gutter, and a tap outside closes it', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'default' })
  await page.getByRole('button', { name: 'Rename widget' }).tap()
  const box = await page.getByRole('dialog').boundingBox()
  expect(box?.x ?? -1).toBeGreaterThanOrEqual(0)
  expect((box?.x ?? 0) + (box?.width ?? 999)).toBeLessThanOrEqual(360)
  await page.getByRole('button', { name: 'Cancel' }).tap()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await page.getByRole('button', { name: 'Rename widget' }).tap()
  await expect(page.getByRole('dialog')).toBeVisible()
  await settled(page)
  await page.touchscreen.tap(10, 730)
  await expect(page.getByRole('dialog')).toHaveCount(0)
})

for (const state of ['default', 'no-description', 'small']) {
  test(`${state} is accessible, closed and open`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
    await expectAccessible(page, { piece, state }, { prepare: open })
  })
}

/** ConfirmDialog: the server's preview, what it affects, the passkey line, then Cancel or Confirm. */
import { expect, test, type Page } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'
import { settled } from '../support/settle'

const piece = 'confirm-dialog'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

async function open(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Remove widget' }).click()
  await expect(page.getByRole('dialog', { name: 'Remove widget' })).toBeVisible()
  await settled(page)
}

test('preview: the summary describes the dialog, and Escape sends nothing', async ({ page }) => {
  await openPiece(page, { piece, state: 'preview' })
  await page.getByRole('button', { name: 'Remove widget' }).focus()
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog', { name: 'Remove widget' })
  await expect(dialog).toHaveAttribute('data-confirm-fixture', 'true')
  await expect(dialog.locator('[data-preview-summary]')).toHaveText('Remove Blue widget.')
  await expect(dialog).toHaveAccessibleDescription('Remove Blue widget.')
  await expect(dialog.locator('[data-affected]')).toHaveCount(0)
  await expect(dialog.locator('[data-step-up]')).toHaveCount(0)
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)
  await expect(page.getByRole('status')).toHaveText('Nothing changed.')
  await expect(page.getByRole('button', { name: 'Remove widget' })).toBeFocused()
})

test('Cancel from the keyboard sends nothing', async ({ page }) => {
  await openPiece(page, { piece, state: 'preview' })
  await open(page)
  await page.getByRole('button', { name: 'Cancel' }).focus()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByRole('status')).toHaveText('Nothing changed.')
})

test('affected: a labelled list, with its count as a heading', async ({ page }) => {
  await openPiece(page, { piece, state: 'affected' })
  await open(page)
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByRole('heading', { level: 3, name: 'Affects 3' })).toBeVisible()
  const list = dialog.getByRole('list', { name: 'Affected' })
  await expect(list).toHaveAttribute('data-affected', 'true')
  await expect(list.getByRole('listitem')).toHaveCount(3)
})

test('affected: long names wrap inside the dialog at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'affected' })
  await page.getByRole('button', { name: 'Remove widget' }).tap()
  const dialog = await page.getByRole('dialog').boundingBox()
  const item = await page.getByRole('listitem').last().boundingBox()
  expect((item?.x ?? 0) + (item?.width ?? 999)).toBeLessThanOrEqual((dialog?.x ?? 0) + (dialog?.width ?? 0))
})

test('step-up: says the change asks for a passkey', async ({ page }) => {
  await openPiece(page, { piece, state: 'step-up' })
  await open(page)
  await expect(page.locator('[data-step-up]')).toHaveText('This change asks for your passkey.')
})

test('denied: the refusal shows inside the dialog, which stays open', async ({ page }) => {
  await openPiece(page, { piece, state: 'denied' })
  await open(page)
  await page.getByRole('button', { name: 'Remove', exact: true }).focus()
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog')
  await expect(dialog.getByRole('alert')).toHaveText('Denied: remove_widget (no_grant)')
  await expect(dialog).toBeVisible()
})

test('done: Confirm closes it and the page says so', async ({ page }) => {
  await openPiece(page, { piece, state: 'done' })
  await open(page)
  await page.getByRole('button', { name: 'Remove', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByRole('status')).toHaveText('Widget removed.')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await settled(page)
})

const confirmDenied = async (page: Page): Promise<void> => {
  await open(page)
  await page.getByRole('button', { name: 'Remove', exact: true }).click()
  await expect(page.getByRole('dialog').getByRole('alert')).toBeVisible()
}

const confirmDone = async (page: Page): Promise<void> => {
  await open(page)
  await page.getByRole('button', { name: 'Remove', exact: true }).click()
  await expect(page.getByRole('status')).toHaveText('Widget removed.')
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await settled(page)
}

for (const state of ['preview', 'affected', 'step-up']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state }, { prepare: open })
  })
}

test('denied is accessible', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'denied' }, { prepare: confirmDenied })
})

test('done is accessible', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'done' }, { prepare: confirmDone })
})

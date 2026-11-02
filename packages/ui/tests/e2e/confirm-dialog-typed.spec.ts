/**
 * ConfirmDialog `typedConfirm` (spec 065 T027): Confirm stays off until the word is typed, Enter
 * confirms once it matches, and each opening starts with an empty field.
 */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'confirm-dialog-typed'

test('Confirm waits for the typed word; Enter then confirms', async ({ page }) => {
  await openPiece(page, { piece, state: 'open' })
  const dialog = page.getByRole('dialog', { name: 'Delete 12 pods?' })
  const field = dialog.getByRole('textbox', { name: 'Type 12 to confirm' })
  const confirm = dialog.getByRole('button', { name: 'Delete' })
  await expect(confirm).toBeDisabled()

  await field.fill('1')
  await field.press('Enter')
  await expect(dialog).toBeVisible()
  await expect(confirm).toBeDisabled()

  await field.fill(' 12 ')
  await expect(confirm).toBeEnabled()
  await field.press('Enter')
  await expect(dialog).toBeHidden()
  await expect(page.locator('[data-confirmed]')).toHaveText('1')
})

test('a new opening starts empty; Cancel and Escape confirm nothing', async ({ page }) => {
  await openPiece(page, { piece, state: 'closed' })
  const open = page.getByRole('button', { name: 'Delete 12 pods' })
  await open.click()
  const dialog = page.getByRole('dialog', { name: 'Delete 12 pods?' })
  const field = dialog.getByRole('textbox', { name: 'Type 12 to confirm' })
  await field.fill('12')
  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()

  await open.click()
  await expect(field).toHaveValue('')
  await expect(dialog.getByRole('button', { name: 'Delete' })).toBeDisabled()
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.locator('[data-confirmed]')).toHaveText('0')
})

test('the dialog is accessible, empty and armed', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'open' })
  await expectAccessible(
    page,
    { piece, state: 'open' },
    {
      prepare: async (p) => {
        await p.getByRole('textbox', { name: 'Type 12 to confirm' }).fill('12')
      },
    },
  )
})

test('closed is accessible', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'closed' })
})

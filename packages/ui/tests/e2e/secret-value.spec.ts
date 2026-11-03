/** SecretValue and SecretInput: a secret stays out of the page until asked for, then goes again. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'secret-value'
const SECRET = 'wsk_live_4f8a2c9e7b1d3f5a6c8e0b2d4f6a8c0e'

test('reveal: only after step-up, and it hides itself again', async ({ page }) => {
  await openPiece(page, { piece, state: 'reveal' })
  const value = page.locator('[data-secret="key"]')
  await expect(value).toHaveAccessibleName('Widget signing key')
  expect(await page.content()).not.toContain(SECRET)

  // Backing out of the step-up leaves it hidden.
  await page.getByRole('button', { name: 'Reveal widget signing key' }).click()
  await page.getByRole('button', { name: 'Cancel' }).click()
  await expect(value).toHaveAttribute('data-revealed', 'false')
  expect(await page.content()).not.toContain(SECRET)

  await page.getByRole('button', { name: 'Reveal widget signing key' }).click()
  await page.getByRole('button', { name: 'Confirm' }).click()
  await expect(value).toHaveText(SECRET)
  await expect(page.getByRole('status').filter({ hasText: 'hides after 3 seconds' })).toHaveCount(1)
  await expect(value).toHaveAttribute('data-revealed', 'false', { timeout: 6000 })
  expect(await page.content()).not.toContain(SECRET)
})

test('reveal: Hide from the keyboard takes it away at once', async ({ page }) => {
  await openPiece(page, { piece, state: 'reveal' })
  await page.getByRole('button', { name: 'Reveal widget signing key' }).click()
  await page.getByRole('button', { name: 'Confirm' }).click()
  const hide = page.getByRole('button', { name: 'Hide widget signing key' })
  await hide.focus()
  await page.keyboard.press('Enter')
  await expect(page.locator('[data-secret="key"]')).toHaveAttribute('data-revealed', 'false')
})

test('failed: says why the reveal did not happen', async ({ page }) => {
  await openPiece(page, { piece, state: 'failed' })
  await page.getByRole('button', { name: 'Reveal widget token' }).click()
  await expect(page.getByRole('alert')).toHaveText('Could not reveal the widget token: Step-up was refused.')
})

test('input: typed as dots, Show to check it', async ({ page }) => {
  await openPiece(page, { piece, state: 'input' })
  const input = page.getByLabel('New value')
  await input.fill('s3cret')
  await expect(input).toHaveAttribute('type', 'password')
  await page.getByRole('button', { name: 'Show' }).click()
  await expect(input).toHaveAttribute('type', 'text')
  await expect(input).toHaveValue('s3cret')
})

test('reveal: no accessibility violations, hidden and shown', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'reveal' })
  await expectAccessible(
    page,
    { piece, state: 'reveal' },
    {
      prepare: async (p) => {
        await p.getByRole('button', { name: 'Reveal widget signing key' }).click()
        await p.getByRole('button', { name: 'Confirm' }).click()
      },
    },
  )
})

test('failed: no accessibility violations once the error shows', async ({ page }) => {
  await expectAccessible(
    page,
    { piece, state: 'failed' },
    {
      prepare: async (p) => {
        await p.getByRole('button', { name: 'Reveal widget token' }).click()
        await expect(p.getByRole('alert')).toBeVisible()
      },
    },
  )
})

test('input: no accessibility violations', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'input' })
})

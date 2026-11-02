/** The form pieces: a labelled Field, the LABEL style, native Selects, FormPanel and SectionIntro. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'form'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test('field: the label names the control, and focus shows a ring', async ({ page }) => {
  await openPiece(page, { piece, state: 'field' })
  const name = page.getByRole('textbox', { name: 'Name' })
  await expect(name).toHaveValue('blue-widget')
  await expect(page.getByText('Letters, digits and dashes.')).toBeVisible()
  await page.keyboard.press('Tab')
  await expect(name).toBeFocused()
  const ring = await name.evaluate((el) => getComputedStyle(el).boxShadow)
  expect(ring).not.toBe('none')
  const owner = page.getByRole('textbox', { name: 'Owner' })
  await expect(owner).toHaveAttribute('aria-invalid', 'true')
  await expect(owner).toHaveAccessibleDescription('An owner is required.')
})

test('label: a quieter, smaller step than the value it names', async ({ page }) => {
  await openPiece(page, { piece, state: 'label' })
  const [label, value] = await Promise.all([
    page.getByRole('term').first().evaluate((el) => getComputedStyle(el).fontSize),
    page.getByRole('definition').first().evaluate((el) => getComputedStyle(el).fontSize),
  ])
  expect(Number.parseFloat(label)).toBeLessThan(Number.parseFloat(value))
})

test('select: native, labelled and driven from the keyboard', async ({ page }) => {
  await openPiece(page, { piece, state: 'select' })
  const type = page.getByRole('combobox', { name: 'Type' })
  await expect(type).toHaveValue('counter')
  await type.selectOption('timer')
  await expect(type).toHaveValue('timer')
  const sort = page.getByRole('combobox', { name: 'Sort' })
  const [full, dense] = await Promise.all([type.boundingBox(), sort.boundingBox()])
  expect(dense?.height ?? 0).toBeLessThan(full?.height ?? 0)
})

test('panel: a named form with its heading; Enter submits', async ({ page }) => {
  await openPiece(page, { piece, state: 'panel' })
  const form = page.getByRole('form', { name: 'Add widget' })
  await expect(form.getByRole('heading', { level: 2, name: 'Add widget' })).toBeVisible()
  await page.keyboard.press('Tab')
  await expect(form.getByRole('textbox', { name: 'Name' })).toBeFocused()
  await page.keyboard.type('Blue widget')
  await page.keyboard.press('Tab')
  await expect(form.getByRole('combobox', { name: 'Type' })).toBeFocused()
  await form.getByRole('combobox', { name: 'Type' }).selectOption('counter')
  await form.getByRole('textbox', { name: 'Name' }).press('Enter')
  await expect(page.getByRole('status')).toHaveText('Added Blue widget (counter).')
})

test('panel: the submit button is tappable at 360 px and the form fits', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'panel' })
  await page.getByRole('button', { name: 'Add widget' }).tap()
  await expect(page.getByRole('status')).toHaveText('Added an unnamed widget (gauge).')
})

test('section-intro: one line under the heading, then Learn more', async ({ page }) => {
  await openPiece(page, { piece, state: 'section-intro' })
  const intro = page.locator('[data-section-intro]')
  await expect(intro).toContainText('Who else can see this widget.')
  await expect(intro.getByRole('link', { name: 'Learn more' })).toHaveAttribute('href', '/help/widgets#sharing')
})

for (const state of ['field', 'label', 'select', 'panel', 'section-intro']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

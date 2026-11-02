/** TermTip: an info button after a glossary term, opened by focus, hover or tap; Escape closes. */
import { expect, test, type Page } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'
import { settled } from '../support/settle'

const piece = 'term-tip'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

const TEXT = 'A thing the harness lists, so a piece has something to show.'

const trigger = (page: Page, term = 'widget') => page.getByRole('button', { name: `About ${term}` })

test('guided: focus opens it, the trigger is described by it, and Escape closes it', async ({ page }) => {
  await openPiece(page, { piece, state: 'guided' })
  await expect(trigger(page)).toHaveAttribute('data-infotip', 'widget')
  // The heading's name stays the term alone.
  await expect(page.getByRole('heading', { level: 2, name: 'Widgets', exact: true })).toBeVisible()

  await page.keyboard.press('Tab')
  await expect(trigger(page)).toBeFocused()
  const note = page.getByRole('note')
  await expect(note).toBeVisible()
  await expect(note).toHaveAttribute('data-infotip-content', 'widget')
  await expect(note).toContainText(TEXT)
  await expect(trigger(page)).toHaveAccessibleDescription(new RegExp(TEXT.replace('.', '\\.')))
  await expect(note.getByRole('link', { name: 'Learn more' })).toHaveAttribute('href', '/help/widgets')

  await page.keyboard.press('Escape')
  await expect(note).toBeHidden()
  await expect(trigger(page)).toBeFocused()
})

test('guided: hover opens it and leaving closes it; a click pins it until a second click', async ({ page }) => {
  await openPiece(page, { piece, state: 'guided' })
  const note = page.getByRole('note')
  await trigger(page).hover()
  await expect(note).toBeVisible()
  await page.mouse.move(0, 0)
  await expect(note).toBeHidden()

  await trigger(page).click()
  await expect(note).toBeVisible()
  await page.mouse.move(0, 0)
  await page.waitForTimeout(300)
  await expect(note).toBeVisible()
  await trigger(page).click()
  await expect(note).toBeHidden()
})

test('guided: at 360 px a tap opens it and a tap elsewhere closes it', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'guided' })
  const note = page.getByRole('note')
  await trigger(page).tap()
  await expect(note).toBeVisible()
  const box = await note.boundingBox()
  expect(box).not.toBeNull()
  expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(360)
  await settled(page)
  await page.touchscreen.tap(180, 700)
  await expect(note).toBeHidden()
})

test('plain: an entry with no guide has no Learn more', async ({ page }) => {
  await openPiece(page, { piece, state: 'plain' })
  await trigger(page, 'gadget').focus()
  const note = page.getByRole('note')
  await expect(note).toHaveText('Another thing, with no guide.')
  await expect(note.getByRole('link')).toHaveCount(0)
})

test('unknown: a term the glossary lacks renders nothing', async ({ page }) => {
  await openPiece(page, { piece, state: 'unknown' })
  await expect(page.getByRole('heading', { name: 'Sprockets' })).toBeVisible()
  await expect(page.getByRole('button')).toHaveCount(0)
})

const open = async (page: Page) => {
  await page.getByRole('button', { name: /^About / }).focus()
  await expect(page.getByRole('note')).toBeVisible()
  await settled(page)
}

for (const state of ['guided', 'plain']) {
  test(`${state} is accessible, closed and open`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
    await expectAccessible(page, { piece, state }, { prepare: open })
  })
}

test('unknown is accessible', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'unknown' })
})

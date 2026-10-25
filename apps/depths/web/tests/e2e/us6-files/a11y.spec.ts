/**
 * Spec 053, the volume explorer by keyboard alone (T069), against the fake backend: Tab reaches
 * the rows, the arrows move between them, Enter opens one, F2 renames and the actions menu opens
 * from the keyboard; focus comes back where it was after every dialog; and the save and transfer
 * lines are live regions a screen reader announces.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'

const listing = (page: Page) => page.locator('[data-pane="listing"]')
const row = (page: Page, name: string) => listing(page).locator(`[data-entry="${name}"]`)
const link = (page: Page, name: string) => row(page, name).locator('[data-row-link]')
const fileView = (page: Page) => page.locator('[data-pane="file"]')
const fileDialog = (page: Page) => page.locator('[data-file-op]')
const nameDialog = (page: Page) => page.locator('[data-name-dialog]')

function seed(backend: FakeBackend): string {
  backend.stacks = f.stackList([f.stack('web')])
  const data = backend.files.addMount('web', { source: 'web_data' })
  backend.files.write('web', data, 'a.txt', 'alpha\n')
  backend.files.write('web', data, 'b.txt', 'beta\n')
  backend.files.special('web', data, 'fifo')
  backend.files.write('web', data, 'c.txt', 'gamma\n')
  return data
}

/** The `data-entry` of the row holding the focused element, or null. */
const focusedEntry = (page: Page) =>
  page.evaluate(() => document.activeElement?.closest('[data-entry]')?.getAttribute('data-entry') ?? null)

/** Tabs forward until a row link has focus. */
async function tabToRows(page: Page): Promise<void> {
  for (let i = 0; i < 40; i++) {
    await page.keyboard.press('Tab')
    if (await page.evaluate(() => document.activeElement?.hasAttribute('data-row-link') ?? false)) return
  }
  throw new Error('Tab never reached a row')
}

test('Tab reaches one row, the arrows move, Enter opens', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)
  await expect(row(page, 'c.txt')).toBeVisible()
  await tabToRows(page)
  expect(await focusedEntry(page)).toBe('a.txt')
  // The arrows skip the special file, which does not open.
  await page.keyboard.press('ArrowDown')
  expect(await focusedEntry(page)).toBe('b.txt')
  await page.keyboard.press('ArrowDown')
  expect(await focusedEntry(page)).toBe('c.txt')
  await page.keyboard.press('ArrowUp')
  expect(await focusedEntry(page)).toBe('b.txt')
  await page.keyboard.press('End')
  expect(await focusedEntry(page)).toBe('c.txt')
  await page.keyboard.press('Home')
  expect(await focusedEntry(page)).toBe('a.txt')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(page).toHaveURL(new RegExp(`/files/${data}/b\\.txt$`))
  await expect(fileView(page).locator('[data-code-editor] .cm-content')).toContainText('beta')
})

test('F2 renames, and Escape puts focus back on the row', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)
  await link(page, 'b.txt').focus()
  await page.keyboard.press('F2')
  await expect(nameDialog(page)).toHaveAttribute('data-name-dialog', 'rename')
  const input = nameDialog(page).getByRole('textbox')
  await expect(input).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(nameDialog(page)).toHaveCount(0)
  await expect(link(page, 'b.txt')).toBeFocused()

  // And through to the end: rename by keyboard, cancel the preview, focus comes back.
  await page.keyboard.press('F2')
  await input.fill('bee.txt')
  await page.keyboard.press('Enter')
  await expect(fileDialog(page)).toHaveAttribute('data-file-op', 'rename')
  await expect(fileDialog(page).locator('[data-confirm]')).toBeEnabled()
  await page.keyboard.press('Escape')
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(link(page, 'b.txt')).toBeFocused()
  expect(backend.files.exists('web', data, 'b.txt')).toBe(true)
})

test('Delete previews from the keyboard, and focus comes back after the dialog', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)
  await link(page, 'a.txt').focus()
  await page.keyboard.press('Delete')
  await expect(fileDialog(page)).toHaveAttribute('data-file-op', 'delete')
  await expect(fileDialog(page).locator('[data-confirm]')).toBeEnabled()
  await page.keyboard.press('Escape')
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(link(page, 'a.txt')).toBeFocused()
})

test('the actions menu opens from the keyboard, also on a row that does not open', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)
  await link(page, 'a.txt').focus()
  await page.keyboard.press('Shift+F10')
  const menu = page.locator('[data-row-menu="a.txt"]')
  await expect(menu).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  await expect(link(page, 'a.txt')).toBeFocused()

  // Rename from the menu: the dialog hands focus back to the row too.
  await page.keyboard.press('Shift+F10')
  await menu.locator('[data-action="rename"]').focus()
  await page.keyboard.press('Enter')
  await expect(nameDialog(page)).toHaveAttribute('data-name-dialog', 'rename')
  await page.keyboard.press('Escape')
  await expect(nameDialog(page)).toHaveCount(0)
  await expect(link(page, 'a.txt')).toBeFocused()

  // The special file does not open, but its menu still has to be reachable without a mouse.
  const trigger = row(page, 'fifo').locator('[data-row-actions]')
  await expect(trigger).toHaveAttribute('tabindex', '0')
  await trigger.focus()
  await page.keyboard.press('Enter')
  await expect(page.locator('[data-row-menu="fifo"]')).toBeVisible()
})

test('the save line is a live region that says Saved., and focus lands in the editor', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}/a.txt`)
  const content = fileView(page).locator('[data-code-editor] .cm-content')
  await expect(content).toContainText('alpha')
  const status = fileView(page).locator('[data-save-status]')
  await expect(status).toHaveAttribute('aria-live', 'polite')
  await expect(status).toHaveAttribute('role', 'status')

  await content.click()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.insertText('omega\n')
  await fileView(page).locator('[data-save]').focus()
  await page.keyboard.press('Enter')
  await expect(fileDialog(page).locator('[data-confirm]')).toBeEnabled()
  await fileDialog(page).locator('[data-confirm]').focus()
  await page.keyboard.press('Enter')
  await expect(fileDialog(page)).toHaveCount(0)
  await expect(status).toHaveText('Saved.')
  // The Save button is disabled once saved, so focus cannot go back there: it goes to the text.
  await expect(content).toBeFocused()
})

test('the transfer line is a live status region', async ({ page, backend }) => {
  const data = seed(backend)
  await page.goto(`/stacks/web/files/${data}`)
  const transfers = listing(page).locator('[data-transfers]')
  await expect(transfers).toHaveAttribute('role', 'status')
  await expect(transfers).toHaveAttribute('aria-live', 'polite')
})

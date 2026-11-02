/**
 * DataTable (spec 065 T025): Keel's table as it was, plus the optional controlled sort, selection
 * and ⋯ row actions, in the table layout and the phone card list.
 */
import { expect, test, type Page } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'data-table'
const order = (page: Page) => page.locator('[data-order]')

test.describe('wide', () => {
  test.use({ viewport: { width: 1280, height: 800 } })

  test('plain is a named table with no sort, selection or menu', async ({ page }) => {
    await openPiece(page, { piece, state: 'plain' })
    const table = page.getByRole('table', { name: 'Widgets' })
    await expect(table.getByRole('row')).toHaveCount(5)
    await expect(table.getByRole('columnheader', { name: 'Name' })).not.toHaveAttribute('aria-sort')
    await expect(table.getByRole('button')).toHaveCount(0)
    await expect(table.getByRole('checkbox')).toHaveCount(0)
  })

  test('empty keeps its headers and says why', async ({ page }) => {
    await openPiece(page, { piece, state: 'empty' })
    const table = page.getByRole('table', { name: 'Widgets' })
    await expect(table.locator('[data-pane-state="empty"]')).toHaveText('No widgets yet.')
    await expect(table.getByRole('checkbox', { name: 'Select all' })).toBeDisabled()
  })

  test('a sortable header cycles ascending, descending, then none, by keyboard', async ({ page }) => {
    await openPiece(page, { piece, state: 'sortable' })
    const table = page.getByRole('table', { name: 'Widgets' })
    const name = table.getByRole('columnheader', { name: 'Name' })
    await expect(name).toHaveAttribute('aria-sort', 'none')
    await expect(table.getByRole('columnheader', { name: 'Owner' })).not.toHaveAttribute('aria-sort')
    await expect(table.getByRole('button', { name: 'Owner' })).toHaveCount(0)

    await page.keyboard.press('Tab')
    await expect(name.getByRole('button', { name: 'Name' })).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(name).toHaveAttribute('aria-sort', 'ascending')
    await expect(order(page)).toHaveText('Bracket,Flange,Gasket,Sprocket')
    await page.keyboard.press('Space')
    await expect(name).toHaveAttribute('aria-sort', 'descending')
    await expect(order(page)).toHaveText('Sprocket,Gasket,Flange,Bracket')
    await page.keyboard.press('Enter')
    await expect(name).toHaveAttribute('aria-sort', 'none')
    await expect(order(page)).toHaveText('Sprocket,Flange,Gasket,Bracket')

    await page.keyboard.press('Tab')
    await page.keyboard.press('Enter')
    await expect(table.getByRole('columnheader', { name: 'Size' })).toHaveAttribute('aria-sort', 'ascending')
    await expect(name).toHaveAttribute('aria-sort', 'none')
    await expect(order(page)).toHaveText('Bracket,Sprocket,Gasket,Flange')
  })

  test('rows check one by one or all at once', async ({ page }) => {
    await openPiece(page, { piece, state: 'selectable' })
    const table = page.getByRole('table', { name: 'Widgets' })
    const all = table.getByRole('checkbox', { name: 'Select all' })
    await all.focus()
    await page.keyboard.press('Tab')
    await expect(table.getByRole('checkbox', { name: 'Select Sprocket' })).toBeFocused()
    await page.keyboard.press('Space')
    await expect(page.locator('[data-checked-keys]')).toHaveText('w1')
    await expect(table.getByRole('row').filter({ hasText: 'Sprocket' })).toHaveAttribute('data-checked', 'true')
    await expect(all).toHaveAttribute('aria-checked', 'mixed')

    await all.click()
    await expect(all).toHaveAttribute('aria-checked', 'true')
    await expect(page.locator('[data-checked-keys]')).toHaveText('w1,w2,w3,w4')
    await all.click()
    await expect(page.locator('[data-checked-keys]')).toHaveText('none')
  })

  test('the ⋯ menu opens by keyboard, runs an action and gives focus back', async ({ page }) => {
    await openPiece(page, { piece, state: 'with-actions' })
    const table = page.getByRole('table', { name: 'Widgets' })
    const trigger = table.getByRole('button', { name: 'Actions for Flange' })
    await trigger.focus()
    await page.keyboard.press('Enter')
    const menu = page.getByRole('menu')
    await expect(menu.getByRole('menuitem')).toHaveText(['Rename', 'Archive', 'Delete'])
    await expect(menu.getByRole('menuitem', { name: 'Archive' })).toHaveAttribute('aria-disabled', 'true')
    await page.keyboard.press('Escape')
    await expect(menu).toBeHidden()
    await expect(trigger).toBeFocused()

    await page.keyboard.press('Enter')
    await page.getByRole('menuitem', { name: 'Delete' }).click()
    await expect(page.locator('[data-last-action]')).toHaveText('Delete Flange')
    await expect(trigger).toBeFocused()
  })
})

test.describe('phone', () => {
  test.use({ viewport: { width: 360, height: 740 }, hasTouch: true })

  test('cards carry the checkbox, the menu and a sort bar', async ({ page }) => {
    await openPiece(page, { piece, state: 'sortable' })
    const list = page.getByRole('list', { name: 'Widgets' })
    await expect(list.getByRole('listitem')).toHaveCount(4)
    const sortBy = page.getByRole('group', { name: 'Sort by' })
    await sortBy.getByRole('button', { name: 'Size' }).tap()
    await expect(order(page)).toHaveText('Bracket,Sprocket,Gasket,Flange')
    await expect(sortBy.getByRole('button', { name: /^Size\W+sorted ascending$/ })).toBeVisible()

    await openPiece(page, { piece, state: 'selectable' })
    await page.getByRole('checkbox', { name: 'Select Gasket' }).tap()
    await expect(page.locator('[data-checked-keys]')).toHaveText('w3')
    await page.getByRole('checkbox', { name: 'Select all' }).tap()
    await expect(page.locator('[data-checked-keys]')).toHaveText('w1,w2,w3,w4')

    await openPiece(page, { piece, state: 'with-actions' })
    await page.getByRole('button', { name: 'Actions for Bracket' }).tap()
    await page.getByRole('menuitem', { name: 'Rename' }).tap()
    await expect(page.locator('[data-last-action]')).toHaveText('Rename Bracket')
  })
})

for (const state of ['plain', 'empty', 'sortable', 'selectable', 'with-actions']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

test('an open row menu is accessible', async ({ page }) => {
  await expectAccessible(
    page,
    { piece, state: 'with-actions' },
    {
      prepare: async (p) => {
        await p.getByRole('button', { name: 'Actions for Sprocket' }).click()
        await expect(p.getByRole('menu')).toBeVisible()
      },
    },
  )
})

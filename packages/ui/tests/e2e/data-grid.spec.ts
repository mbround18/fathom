/**
 * DataGrid (spec 065 T026): the virtualized ARIA grid. Rows by keyboard, sort, selection, the row
 * menu by keyboard, pointer and long press, the empty and failed slots, and 10,000 rows with only
 * the visible ones in the page.
 */
import { expect, test, type Page } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'data-grid'
const grid = (page: Page) => page.getByRole('grid', { name: 'Widgets' })
const marker = (page: Page, name: string) => page.locator(`[data-${name}]`)

test.describe('wide', () => {
  test.use({ viewport: { width: 1280, height: 800 } })

  test('plain is a named grid the keyboard walks and opens', async ({ page }) => {
    await openPiece(page, { piece, state: 'plain' })
    const g = grid(page)
    await expect(g).toHaveAttribute('aria-rowcount', '5')
    await expect(g.locator('[role="row"][data-row-key]')).toHaveCount(4)
    await expect(g.getByRole('checkbox')).toHaveCount(0)
    await expect(g.getByRole('button')).toHaveCount(0)
    await expect(g.getByRole('columnheader', { name: 'Name' })).not.toHaveAttribute('aria-sort')
    await expect(page.getByTestId('widget-grid')).toHaveAttribute('data-columns', 'name,size,owner,note')

    await page.keyboard.press('Tab')
    await expect(g).toBeFocused()
    await page.keyboard.press('ArrowDown')
    await expect(marker(page, 'cursor-key')).toHaveText('w1')
    await page.keyboard.press('End')
    await expect(marker(page, 'cursor-key')).toHaveText('w4')
    const bracket = g.locator('[data-row-key="w4"]')
    await expect(g).toHaveAttribute('aria-activedescendant', (await bracket.getAttribute('id')) ?? '')
    await expect(bracket.locator('[data-cursor]')).toHaveCount(1)
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('Enter')
    await expect(marker(page, 'open-key')).toHaveText('w3')
    await expect(g.locator('[data-row-key="w3"]')).toHaveAttribute('aria-current', 'true')
    await page.keyboard.press('Home')
    await expect(marker(page, 'cursor-key')).toHaveText('w1')

    await g.getByRole('gridcell', { name: 'Flange', exact: true }).click()
    await expect(marker(page, 'open-key')).toHaveText('w2')
  })

  test('a sortable header cycles ascending, descending, then none', async ({ page }) => {
    await openPiece(page, { piece, state: 'full' })
    const g = grid(page)
    const size = g.getByRole('columnheader', { name: 'Size' })
    await expect(size).toHaveAttribute('aria-sort', 'none')
    await expect(g.getByRole('columnheader', { name: 'Owner' })).not.toHaveAttribute('aria-sort')
    const button = size.getByRole('button', { name: 'Size' })
    await button.focus()
    await page.keyboard.press('Enter')
    await expect(size).toHaveAttribute('aria-sort', 'ascending')
    await expect(marker(page, 'order')).toHaveText('Bracket,Sprocket,Gasket,Flange')
    await page.keyboard.press('Enter')
    await expect(size).toHaveAttribute('aria-sort', 'descending')
    await expect(marker(page, 'order')).toHaveText('Flange,Gasket,Sprocket,Bracket')
    await page.keyboard.press('Enter')
    await expect(size).toHaveAttribute('aria-sort', 'none')
    // The header's own keys stay its own: the grid's cursor did not move.
    await expect(marker(page, 'cursor-key')).toHaveText('none')
  })

  test('Space checks the cursor row; the header checks them all', async ({ page }) => {
    await openPiece(page, { piece, state: 'full' })
    const g = grid(page)
    await expect(g).toHaveAttribute('aria-multiselectable', 'true')
    await g.focus()
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Space')
    await expect(marker(page, 'checked-keys')).toHaveText('w2')
    const flange = g.locator('[data-row-key="w2"]')
    await expect(flange).toHaveAttribute('aria-selected', 'true')
    await expect(flange).toHaveAttribute('data-checked', 'true')
    const all = g.getByRole('checkbox', { name: 'Select all' })
    await expect(all).toHaveAttribute('aria-checked', 'mixed')

    await g.getByRole('checkbox', { name: 'Select Gasket' }).click()
    await expect(marker(page, 'checked-keys')).toHaveText('w2,w3')
    // A click on the checkbox checks, it does not open the row.
    await expect(marker(page, 'open-key')).toHaveText('none')
    await all.click()
    await expect(marker(page, 'checked-keys')).toHaveText('w1,w2,w3,w4')
    await all.click()
    await expect(marker(page, 'checked-keys')).toHaveText('none')
  })

  test('the row menu opens by Shift+F10, runs an action and gives focus back', async ({ page }) => {
    await openPiece(page, { piece, state: 'full' })
    const g = grid(page)
    await g.focus()
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Shift+F10')
    const menu = page.getByRole('menu', { name: 'Actions for Flange' })
    await expect(menu.getByRole('menuitem')).toHaveText(['Rename', 'Archive', 'Delete'])
    await expect(menu.getByRole('menuitem', { name: 'Archive' })).toHaveAttribute('aria-disabled', 'true')
    await page.keyboard.press('Escape')
    await expect(menu).toBeHidden()
    await expect(g).toBeFocused()

    await page.keyboard.press('ContextMenu')
    await page.getByRole('menuitem', { name: 'Delete' }).click()
    await expect(marker(page, 'last-action')).toHaveText('Delete Flange')
    await expect(g).toBeFocused()
  })

  test('the ⋯ button and right-click open the same menu', async ({ page }) => {
    await openPiece(page, { piece, state: 'full' })
    const g = grid(page)
    const gasket = g.locator('[data-row-key="w3"]')
    await gasket.hover()
    await gasket.getByRole('button', { name: 'Actions for Gasket' }).click()
    await page.getByRole('menuitem', { name: 'Rename' }).click()
    await expect(marker(page, 'last-action')).toHaveText('Rename Gasket')
    await expect(marker(page, 'open-key')).toHaveText('none')

    await g.getByRole('gridcell', { name: 'Bracket', exact: true }).click({ button: 'right' })
    await page.getByRole('menuitem', { name: 'Archive' }).click()
    await expect(marker(page, 'last-action')).toHaveText('Archive Bracket')
  })

  test('empty and failed fill the body with their slot', async ({ page }) => {
    await openPiece(page, { piece, state: 'empty' })
    await expect(grid(page).locator('[data-grid-status="empty"]')).toHaveText('No widgets yet.')
    await expect(grid(page).getByRole('checkbox', { name: 'Select all' })).toBeDisabled()
    await openPiece(page, { piece, state: 'failed' })
    await expect(grid(page).locator('[data-grid-status="failed"]').getByRole('alert')).toHaveText('Widgets failed to load.')
    await expect(grid(page).locator('[role="row"][data-row-key]')).toHaveCount(0)
  })

  test('10,000 rows keep only the visible ones in the page', async ({ page }) => {
    await openPiece(page, { piece, state: 'ten-thousand' })
    const g = grid(page)
    const rows = g.locator('[role="row"][data-row-key]')
    await expect(g).toHaveAttribute('aria-rowcount', '10001')
    await expect(page.getByTestId('widget-grid')).toHaveAttribute('data-row-count', '10000')
    await expect(rows.first()).toBeVisible()
    expect(await rows.count()).toBeLessThanOrEqual(60)

    await g.focus()
    await page.keyboard.press('End')
    await expect(marker(page, 'cursor-key')).toHaveText('m9999')
    const lastRow = g.locator('[data-row-key="m9999"]')
    await expect(lastRow).toBeInViewport()
    await expect(lastRow).toHaveAttribute('aria-rowindex', '10001')
    expect(await rows.count()).toBeLessThanOrEqual(60)
    await page.keyboard.press('PageUp')
    await expect(marker(page, 'cursor-key')).not.toHaveText('m9999')
    await page.keyboard.press('Home')
    await expect(marker(page, 'cursor-key')).toHaveText('m0')
    await expect(g.locator('[data-row-key="m0"]')).toBeInViewport()

    await g.getByRole('checkbox', { name: 'Select all' }).click()
    await expect(marker(page, 'checked-keys')).toHaveText('10000 rows')
  })
})

test.describe('phone', () => {
  test.use({ viewport: { width: 360, height: 740 }, hasTouch: true })

  test('narrow hides low-priority columns, never squashes them', async ({ page }) => {
    await openPiece(page, { piece, state: 'full' })
    await expect(page.getByTestId('widget-grid')).toHaveAttribute('data-columns', 'name,size,owner')
    await expect(grid(page).getByRole('columnheader', { name: 'Note' })).toHaveCount(0)
  })

  test('a long press opens the row menu with the pressed text', async ({ page }) => {
    await openPiece(page, { piece, state: 'full' })
    const cell = grid(page).locator('[data-row-key="w1"] [data-col="name"]')
    const box = await cell.boundingBox()
    if (!box) throw new Error('no cell box')
    const at = { pointerType: 'touch', isPrimary: true, bubbles: true, clientX: box.x + 10, clientY: box.y + 10 }
    await cell.dispatchEvent('pointerdown', at)
    const menu = page.getByRole('menu', { name: 'Actions for Sprocket' })
    await expect(menu).toBeVisible()
    await expect(menu).toContainText('Sprocket')
    await cell.dispatchEvent('pointerup', at)
    await menu.getByRole('menuitem', { name: 'Rename' }).tap()
    await expect(marker(page, 'last-action')).toHaveText('Rename Sprocket')
    // The press did not also open the row.
    await expect(marker(page, 'open-key')).toHaveText('none')
  })

  test('a long press on a grid with no menu peeks the full text', async ({ page }) => {
    await openPiece(page, { piece, state: 'plain' })
    const cell = grid(page).locator('[data-row-key="w1"] [data-col="name"]')
    const box = await cell.boundingBox()
    if (!box) throw new Error('no cell box')
    await cell.dispatchEvent('pointerdown', { pointerType: 'touch', isPrimary: true, bubbles: true, clientX: box.x + 5, clientY: box.y + 5 })
    await expect(cell.getByRole('tooltip')).toHaveText('Sprocket')
  })
})

for (const state of ['plain', 'full', 'empty', 'failed', 'ten-thousand']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}

test('an open row menu is accessible', async ({ page }) => {
  await expectAccessible(
    page,
    { piece, state: 'full' },
    {
      prepare: async (p) => {
        await grid(p).focus()
        await p.keyboard.press('ArrowDown')
        await p.keyboard.press('Shift+F10')
        await expect(p.getByRole('menu')).toBeVisible()
      },
    },
  )
})

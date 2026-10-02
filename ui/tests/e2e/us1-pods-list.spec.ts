/**
 * T055 (US1 scenarios 2–3): the pods list renders the whole list, filters by name prefix or
 * namespace as you type, and survives list refreshes with the search text and selection intact.
 * Ports tests/gui_pods.rs and tests/gui_search_filter.rs.
 */
import { expect, test } from '@playwright/test'

import { PODS_500, pods } from '../fixtures'
import {
  emit,
  expectNoHorizontalScroll,
  expectRowCount,
  isPhone,
  open,
  row,
  searchBox,
  table,
} from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`
const WEB = PODS_500.filter((p) => p.name.startsWith('web-'))
const PAYMENTS = PODS_500.filter((p) => p.namespace === 'payments')

test.describe('US1 pods list', () => {
  test('renders all 500 pods with name and status visible and no horizontal scroll', async ({
    page,
  }) => {
    await open(page)
    await expectRowCount(page, 500)
    const first = PODS_500[0]
    if (!first) throw new Error('fixture')
    const r = row(page, key(first))
    await expect(r).toBeVisible()
    await expect(r).toContainText(first.name)
    // Status carries a glyph and text, never colour alone.
    await expect(r.locator('[data-col="status"]')).toContainText(first.phase)
    await expect(table(page).getByRole('columnheader', { name: 'Name', exact: true })).toBeVisible()
    await expect(
      table(page).getByRole('columnheader', { name: 'Status', exact: true }),
    ).toBeVisible()
    await expectNoHorizontalScroll(page)
    const body = table(page).locator('[data-table-body]')
    const { sw, cw } = await body.evaluate((el) => ({ sw: el.scrollWidth, cw: el.clientWidth }))
    expect(sw).toBeLessThanOrEqual(cw)
  })

  test('filters case-insensitively by name prefix and namespace; clearing restores', async ({
    page,
  }) => {
    await open(page)
    await expectRowCount(page, 500)

    await searchBox(page).fill('web-')
    await expectRowCount(page, WEB.length)
    await searchBox(page).fill('WEB-')
    await expectRowCount(page, WEB.length)
    // Prefix, not substring: "eb-" matches nothing by name.
    await searchBox(page).fill('eb-')
    await expectRowCount(page, 0)
    await expect(
      page.locator('[data-pane-kind="list"] [data-pane-state="empty"]').first(),
    ).toBeVisible()

    await searchBox(page).fill('payments')
    await expectRowCount(page, PAYMENTS.length)

    await searchBox(page).fill('')
    await expectRowCount(page, 500)
  })

  test('a refresh adds and removes rows without losing search text or selection', async ({
    page,
  }) => {
    await open(page)
    await expectRowCount(page, 500)
    await searchBox(page).fill('web-')
    await expectRowCount(page, WEB.length)

    const keep = WEB[0]
    const gone = WEB.at(-1)
    if (!keep || !gone) throw new Error('fixture')
    const check = row(page, key(keep)).getByRole('checkbox')
    await check.check()
    await expect(check).toBeChecked()

    const added = { ...keep, name: 'web-00000-added', restarts: 0 }
    const next = [...PODS_500.filter((p) => p !== gone), added].sort((a, b) =>
      a.name.localeCompare(b.name),
    )
    await emit(page, pods('kind-a', next, 99))

    await expectRowCount(page, WEB.length)
    await expect(searchBox(page)).toHaveValue('web-')
    await expect(row(page, key(added))).toBeVisible()
    await expect(row(page, key(gone))).toHaveCount(0)
    await expect(row(page, key(keep)).getByRole('checkbox')).toBeChecked()
  })

  test('sorts by a column header', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
    const header = table(page).getByRole('columnheader', { name: 'Name', exact: true })
    await header.click()
    await expect(header).toHaveAttribute('aria-sort', 'ascending')
    await header.click()
    await expect(header).toHaveAttribute('aria-sort', 'descending')
    const last = [...PODS_500].sort((a, b) =>
      b.name.localeCompare(a.name, undefined, { numeric: true, sensitivity: 'base' }),
    )[0]
    if (!last) throw new Error('fixture')
    await expect(table(page).locator('[role="row"][data-row-key]').first()).toHaveAttribute(
      'data-row-key',
      key(last),
    )
  })

  test('uses 44px touch rows on phone', async ({ page }, info) => {
    test.skip(!isPhone(info), 'phone only')
    await open(page)
    await expectRowCount(page, 500)
    const first = PODS_500[0]
    if (!first) throw new Error('fixture')
    const box = await row(page, key(first)).boundingBox()
    expect(box?.height).toBeGreaterThanOrEqual(44)
    const cb = await row(page, key(first)).getByRole('checkbox').boundingBox()
    expect(cb?.width).toBeGreaterThanOrEqual(44)
  })

  test('stays responsive with 10,000 pods', async ({ page }) => {
    await open(page, 'pods-10k')
    await expectRowCount(page, 10_000)
    const rendered = await table(page).locator('[role="row"][data-row-key]').count()
    expect(rendered).toBeLessThan(200)
    const t0 = Date.now()
    await searchBox(page).fill('web-')
    await expectRowCount(page, 1000)
    // The budget guards against a regression that filters 10k rows without virtualisation — that
    // costs tens of seconds, not hundreds of milliseconds. The elapsed time also includes
    // Playwright's own round trips for the fill and the count poll, which stretch under parallel
    // workers on a shared box (measured 2180 ms at four workers), so the bar sits where a real
    // regression is still caught and harness contention is not reported as one.
    expect(Date.now() - t0).toBeLessThan(5000)
  })
})

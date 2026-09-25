/**
 * T054 (US1 scenario 1, FR-016, FR-017): switching context resets and repopulates the lists,
 * late envelopes from the old context never render, and every pane shows a clear state instead
 * of going blank. Ports tests/gui_context_switching.rs.
 */
import { expect, test, type Page } from '@playwright/test'

import { genDeployments } from '../fixtures/gen-pods'
import {
  PODS_500,
  PODS_FORBIDDEN,
  PODS_KIND_B,
  pods500,
  STALE_POD_NAME,
  staleKindA,
  twoContexts,
} from '../fixtures'
import {
  DEPLOYMENTS,
  chooseScope,
  clearSent,
  emit,
  expectNoHorizontalScroll,
  expectRowCount,
  expectScope,
  isPhone,
  listPane,
  open,
  row,
  sent,
  table,
} from './helpers'

async function pickContext(page: Page, name: string) {
  await chooseScope(page, 'Context', name)
}

test.describe('US1 context switching', () => {
  test('selecting kind-b resets and repopulates lists; late kind-a data never renders', async ({
    page,
  }, info) => {
    await open(page)
    await expectRowCount(page, PODS_500.length)
    await expectScope(page, 'Context', 'kind-a')
    await clearSent(page)

    await pickContext(page, 'kind-b')

    await expectScope(page, 'Context', 'kind-b')
    await expectRowCount(page, PODS_KIND_B.length)
    const cmds = await sent(page)
    expect(cmds.filter((c) => c.type === 'SetContext')).toEqual([
      { type: 'SetContext', data: { context: 'kind-b' } },
    ])
    // The fake emits a late kind-a pods batch before ContextChanged; it must be dropped.
    await expect(row(page, `default/${STALE_POD_NAME}`)).toHaveCount(0)

    // And a straggler arriving after the switch is dropped too.
    await emit(page, staleKindA)
    await expectRowCount(page, PODS_KIND_B.length)
    await expect(table(page).getByText(STALE_POD_NAME)).toHaveCount(0)

    // Every visible list resets, not only pods (wide shows deployments in its second list slot).
    if (!isPhone(info) && (await table(page, DEPLOYMENTS).count()) > 0) {
      await expectRowCount(page, genDeployments('kind-b').length, DEPLOYMENTS)
    }
    await expectNoHorizontalScroll(page)
  })

  test('shows Loading while the first list is in flight', async ({ page }) => {
    await open(page, 'manual')
    await emit(page, twoContexts)
    await expect(listPane(page).locator('[data-pane-state="loading"]')).toBeVisible()
    await emit(page, pods500)
    await expectRowCount(page, PODS_500.length)
    await expect(listPane(page).locator('[data-pane-state="loading"]')).toHaveCount(0)
  })

  test('shows NoKubeconfig with the backend error instead of a blank pane', async ({ page }) => {
    await open(page, 'no-kubeconfig')
    const state = page.locator('[data-pane-kind="list"] [data-pane-state="no-kubeconfig"]').first()
    await expect(state).toBeVisible()
    await expect(state).toContainText('failed to infer config')
  })

  test('shows the backend error verbatim when the list cannot be read', async ({ page }) => {
    await open(page, 'pods-error')
    const state = listPane(page).locator('[data-pane-state="error"]')
    await expect(state).toBeVisible()
    await expect(state).toContainText(PODS_FORBIDDEN)
  })

  test('shows Disconnected when the backend goes away', async ({ page }) => {
    await open(page)
    await expectRowCount(page, PODS_500.length)
    await page.evaluate(() => window.__fathomFake?.setBackendGone(true))
    await pickContext(page, 'kind-b')
    await expect(
      page.locator('[data-pane-kind="list"] [data-pane-state="disconnected"]').first(),
    ).toBeVisible()
  })
})

/**
 * Promo scenes, captured live from the `fathom-dev` sandbox (read-only; see helpers.ts). Each
 * scene writes one PNG to promo-out/shots/; `compose.spec.ts` runs after and builds the PDF.
 *
 * Dialogs (scale, bulk delete) are opened and photographed, then dismissed: nothing is confirmed.
 */
import type { Page } from '@playwright/test'

import { check } from '../e2e/helpers'
import { DEPLOYMENTS, inspector, key, row, showList, table } from '../live/helpers'

import { expect, LIVE_NAMESPACE, listPods, redactions, shoot, test } from './helpers'

/** A live pod of `app=<app>` (the first by name, so reruns pick the same one). */
function podOf(app: string): string {
  const names = listPods(`app=${app}`)
    .filter((p) => !p.metadata.deletionTimestamp)
    .map((p) => p.metadata.name)
    .sort()
  const name = names[0]
  if (!name) {
    throw new Error(
      `no live pod with app=${app} in ${LIVE_NAMESPACE}; ` +
        `run 'FATHOM_SANDBOX_CONTEXT=<ctx> scripts/sandbox.sh reset'`,
    )
  }
  return name
}

async function ready(page: Page, app: { open(): Promise<void>; selectContext(): Promise<void>; selectNamespace(n: string): Promise<void> }): Promise<void> {
  await app.open()
  await app.selectContext()
  await app.selectNamespace(LIVE_NAMESPACE)
  // Every sandbox workload is listed before anything is photographed.
  await expect
    .poll(async () => Number(await table(page).getAttribute('data-row-count')), {
      timeout: 30_000,
    })
    .toBeGreaterThanOrEqual(listPods().length)
}

async function openInspector(page: Page, podName: string): Promise<void> {
  await row(page, key(podName)).click()
  await expect(inspector(page).getByRole('heading', { name: podName })).toBeVisible()
}

test.describe('wide', () => {
  test.use({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 2 })

  test('overview with inspector', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    await shoot(page, 'wide-inspector', pairs)
  })

  test('scale dialog', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await showList(page, DEPLOYMENTS, 'Deployments')
    await row(page, key('scale-me'), DEPLOYMENTS).click({ button: 'right' })
    await page.getByTestId('row-action-menu').getByTestId('action-scale').click()
    const dialog = page.getByTestId('scale-dialog')
    await dialog.getByLabel('Replicas', { exact: true }).fill('3')
    await expect(dialog.getByTestId('scale-submit')).toHaveText('Scale 1 → 3')
    await shoot(page, 'wide-scale', pairs)
    await dialog.getByTestId('scale-cancel').click()
  })

  test('bulk delete confirmation', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    for (const a of ['crashloop', 'bad-image', 'unschedulable']) await check(page, key(podOf(a)))
    await expect(page.getByTestId('selection-count')).toContainText('3 selected')
    await page.getByTestId('bulk-delete').click()
    const dialog = page.getByTestId('confirm-dialog')
    await expect(dialog.getByTestId('confirm-item')).toHaveCount(3)
    await shoot(page, 'wide-confirm', pairs)
    await dialog.getByTestId('confirm-cancel').click()
  })

  test('flux panel', async ({ app, page, toolPane }) => {
    const pairs = redactions()
    await ready(page, app)
    await toolPane.open(page, 'flux')
    await shoot(page, 'wide-flux', pairs)
  })

  test('hubble panel', async ({ app, page, toolPane }) => {
    const pairs = redactions()
    await ready(page, app)
    await toolPane.open(page, 'hubble')
    await shoot(page, 'wide-hubble', pairs)
  })
})

test.describe('ultra', () => {
  test.use({ viewport: { width: 3840, height: 1600 }, deviceScaleFactor: 1 })

  test('six-slot overview', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    await shoot(page, 'ultra', pairs)
  })
})

test.describe('tablet', () => {
  test.use({ viewport: { width: 820, height: 1180 }, deviceScaleFactor: 2, hasTouch: true })

  test('list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await shoot(page, 'tablet', pairs)
  })
})

test.describe('phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, hasTouch: true })

  test('list', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await shoot(page, 'phone-list', pairs)
  })

  test('inspector', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await openInspector(page, podOf('web'))
    await shoot(page, 'phone-inspector', pairs)
  })

  test('navigator', async ({ app, page }) => {
    const pairs = redactions()
    await ready(page, app)
    await page.getByRole('button', { name: 'Open navigator' }).click()
    await expect(page.getByRole('navigation', { name: 'Sections' })).toBeVisible()
    await shoot(page, 'phone-nav', pairs)
  })
})

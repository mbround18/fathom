/**
 * T057 (US1 scenario 5): the global namespace filter carries from Pods into Deployments (and,
 * once its descriptor lands, ConfigMaps).
 */
import { expect, test } from '@playwright/test'

import { genDeployments } from '../fixtures/gen-pods'
import { PODS_500 } from '../fixtures'
import { DEPLOYMENTS, expectRowCount, navigateTo, open, sent, table } from './helpers'

const IN_PAYMENTS = PODS_500.filter((p) => p.namespace === 'payments').length
const DEPLOY_PAYMENTS = genDeployments('kind-a').filter((d) => d.namespace === 'payments').length

test.describe('US1 namespace filter', () => {
  test('filtering to payments applies to pods, then to deployments', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)

    await page.getByRole('combobox', { name: 'Namespace' }).click()
    await page.getByRole('option', { name: 'payments' }).click()

    await expect(page.getByRole('combobox', { name: 'Namespace' })).toContainText('payments')
    await expectRowCount(page, IN_PAYMENTS)
    expect((await sent(page)).filter((c) => c.type === 'SetNamespaceFilter').at(-1)).toEqual({
      type: 'SetNamespaceFilter',
      data: { namespaces: ['payments'] },
    })

    await navigateTo(page, 'Deployments')
    await expectRowCount(page, DEPLOY_PAYMENTS, DEPLOYMENTS)
    const namespaces = await table(page, DEPLOYMENTS)
      .locator('[role="row"][data-row-key]')
      .evaluateAll((rows) => rows.map((r) => r.getAttribute('data-row-key')?.split('/')[0]))
    expect(new Set(namespaces)).toEqual(new Set(['payments']))
    await expect(page.getByRole('combobox', { name: 'Namespace' })).toContainText('payments')

    await navigateTo(page, 'Pods')
    await expectRowCount(page, IN_PAYMENTS)
  })

  test.fixme('the filter carries into ConfigMaps', async () => {
    // Needs the configmaps descriptor and a RefreshConfigMaps reply in the fake (T073).
  })
})

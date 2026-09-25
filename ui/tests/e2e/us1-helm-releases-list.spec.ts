/**
 * T021 (US1 scenario "browse Helm releases"): the Helm Releases section lists releases read
 * from Helm's own release-storage objects, with the expected columns, and respects the active
 * namespace filter — no `cluster()` wiring for `helm/releases` in `tests/fixtures/scenarios.ts`,
 * so this follows `us1-applications-list.spec.ts`'s hand-built-fixture convention.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, list, nsRoute } from '../fixtures/builders'
import {
  chooseScope,
  emit,
  expectRowCount,
  navigateTo,
  open,
  row,
  sent,
  sentTypes,
  table,
} from './helpers'

const HELM = 'helm/releases'

const REDIS = {
  name: 'redis',
  namespace: 'cache',
  chart: 'redis',
  chart_version: '18.6.1',
  app_version: '7.2.4',
  revision: 3,
  status: 'Deployed' as const,
  updated: '2024-01-15T10:00:00Z',
  degraded: false,
}

const WEB = {
  name: 'web',
  namespace: 'payments',
  chart: 'web-app',
  chart_version: '2.1.0',
  app_version: '1.4.0',
  revision: 7,
  status: 'Failed' as const,
  updated: '2024-02-20T08:30:00Z',
  degraded: false,
}

async function goToHelmReleases(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Helm Releases', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshHelmReleases')
}

test.describe('US1 Helm releases list', () => {
  test('T021: lists Helm releases with the expected columns', async ({ page }, info) => {
    const fullColumns = info.project.name === 'chromium-desktop'

    await goToHelmReleases(page)
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [REDIS, WEB] }),
      ]),
    )
    await expectRowCount(page, 2, HELM)

    const redisRow = row(page, `${REDIS.namespace}/${REDIS.name}`, HELM)
    await expect(redisRow).toContainText('redis')
    const webRow = row(page, `${WEB.namespace}/${WEB.name}`, HELM)
    await expect(webRow).toContainText('web')

    if (fullColumns) {
      await expect(redisRow).toContainText('cache')
      await expect(redisRow).toContainText('18.6.1')
      await expect(redisRow).toContainText('7.2.4')
      await expect(webRow).toContainText('payments')
      await expect(webRow).toContainText('Failed')
    }
  })

  test('T021: respects the active namespace filter', async ({ page }) => {
    await goToHelmReleases(page)
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [REDIS, WEB] }),
      ]),
    )
    await expectRowCount(page, 2, HELM)

    await chooseScope(page, 'Namespace', 'payments')
    expect((await sent(page)).filter((c) => c.type === 'SetNamespaceFilter').at(-1)).toEqual({
      type: 'SetNamespaceFilter',
      data: { namespaces: ['payments'] },
    })

    await emit(
      page,
      batch([list(2, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [WEB] })]),
    )
    await expectRowCount(page, 1, HELM)
    await expect(table(page, HELM)).toContainText('web')
    await expect(table(page, HELM)).not.toContainText('redis')
  })
})

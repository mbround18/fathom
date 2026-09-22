/**
 * T038 (US3 "inspect full release detail"): a selected release's inspector distinguishes
 * user-supplied from effective values, shows the rendered manifest and revision history, and
 * a degraded release (undecodable payload) still renders from storage-object labels alone
 * (constitution: never a crash or silent omission).
 */
import { expect, test, type Page } from '@playwright/test'

import type { HelmReleaseDetails, HelmReleaseSummary } from '../../src/protocol/types'
import { batch, list, nsRoute, read } from '../fixtures/builders'
import { emit, inspector, navigateTo, open, row, sent, sentTypes } from './helpers'

const HELM = 'helm/releases'

const REDIS: HelmReleaseSummary = {
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

const DEGRADED: HelmReleaseSummary = {
  name: 'legacy-app',
  namespace: 'default',
  chart: 'unknown',
  chart_version: 'unknown',
  app_version: null,
  revision: 1,
  status: 'Unknown' as const,
  updated: '2024-01-01T00:00:00Z',
  degraded: true,
}

const REDIS_DETAILS: HelmReleaseDetails = {
  name: REDIS.name,
  namespace: REDIS.namespace,
  chart: REDIS.chart,
  chart_version: REDIS.chart_version,
  app_version: REDIS.app_version,
  revision: REDIS.revision,
  status: REDIS.status,
  updated: REDIS.updated,
  description: 'Upgrade complete',
  values_user_supplied: 'replicaCount: 3',
  values_effective: 'replicaCount: 3\nimage:\n  repository: bitnami/redis\n  tag: 7.2.4',
  manifest: 'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis',
  revisions: [
    { revision: 3, status: 'Deployed', description: 'Upgrade complete', updated: REDIS.updated, app_version: '7.2.4', chart_version: '18.6.1' },
    { revision: 2, status: 'Superseded', description: 'Upgrade complete', updated: '2024-01-10T10:00:00Z', app_version: '7.2.0', chart_version: '18.5.0' },
    { revision: 1, status: 'Superseded', description: 'Install complete', updated: '2024-01-01T10:00:00Z', app_version: '7.1.0', chart_version: '18.0.0' },
  ],
  origin: null,
  related: [
    {
      section: 'workloads/pods',
      kind: 'Pod',
      namespace: REDIS.namespace,
      name: 'redis-master-0',
      relation: 'owns',
      status_hint: 'Running',
    },
  ],
  degraded: false,
}

const DEGRADED_DETAILS: HelmReleaseDetails = {
  name: DEGRADED.name,
  namespace: DEGRADED.namespace,
  chart: DEGRADED.chart,
  chart_version: DEGRADED.chart_version,
  app_version: DEGRADED.app_version,
  revision: DEGRADED.revision,
  status: DEGRADED.status,
  updated: DEGRADED.updated,
  description: null,
  values_user_supplied: '',
  values_effective: '',
  manifest: '',
  revisions: [],
  origin: null,
  related: [],
  degraded: true,
}

async function selectRelease(page: Page, r: typeof REDIS): Promise<void> {
  await open(page)
  await navigateTo(page, 'Helm Releases', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshHelmReleases')
  await emit(page, batch([list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [r] })]))
  await row(page, `${r.namespace}/${r.name}`, HELM).click()
  await expect
    .poll(() => sent(page))
    .toEqual(
      expect.arrayContaining([
        { type: 'SelectHelmRelease', data: { namespace: r.namespace, name: r.name } },
      ]),
    )
}

test.describe('US3 Helm release full detail', () => {
  test('T038: distinguishes user-supplied from effective values, shows manifest and revision history', async ({
    page,
  }) => {
    await selectRelease(page, REDIS)
    await emit(
      page,
      batch([read(2, nsRoute('kind-a', HELM, REDIS.namespace), { type: 'HelmReleaseDetailsUpdated', data: REDIS_DETAILS })]),
    )

    const insp = inspector(page)
    await expect(insp).toContainText('replicaCount: 3')
    await expect(insp).toContainText('bitnami/redis')
    await expect(insp).toContainText('kind: Service')
    await expect(insp).toContainText('#3')
    await expect(insp).toContainText('#2')
    await expect(insp).toContainText('#1')
    await expect(insp).toContainText('redis-master-0')
    await expect(insp).toContainText('owns')
  })

  test('T038: a degraded release still renders from storage-object labels alone', async ({ page }) => {
    await selectRelease(page, DEGRADED)
    await emit(
      page,
      batch([read(2, nsRoute('kind-a', HELM, DEGRADED.namespace), { type: 'HelmReleaseDetailsUpdated', data: DEGRADED_DETAILS })]),
    )

    const insp = inspector(page)
    await expect(insp).toContainText('legacy-app')
    await expect(insp).toContainText('release storage undecodable')
  })
})

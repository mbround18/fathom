/** Batch G: read-only Argo CD pane driven by the generic CRD catalog. */
import { expect, test } from '@playwright/test'

import type { CrdCatalogEntry } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { clearSent, emit, navigateTo, open, sent } from './helpers'

const entry = (kind: string, plural: string, denied = false): CrdCatalogEntry => ({
  group: 'argoproj.io',
  kind,
  plural,
  versions: ['v1alpha1'],
  scope: 'Namespaced',
  permission_denied: denied,
})

const app = (name: string, sync: string, health: string) =>
  JSON.stringify({
    apiVersion: 'argoproj.io/v1alpha1',
    kind: 'Application',
    metadata: { namespace: 'argocd', name },
    status: { sync: { status: sync }, health: { status: health } },
  })

test.describe('Argo CD pane', () => {
  test('not detected when the CRDs are absent', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Argo', 'Integrations')
    await emit(page, batch([system(1, { type: 'CrdCatalogUpdated', data: { entries: [] } })]))
    await expect(page.getByTestId('argo-not-detected')).toContainText('Argo CD not detected')
  })

  test('lists Applications read-only and opens one as a custom resource', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Argo', 'Integrations')
    await clearSent(page)
    await emit(
      page,
      batch([
        system(1, {
          type: 'CrdCatalogUpdated',
          data: {
            entries: [
              entry('Application', 'applications'),
              entry('AppProject', 'appprojects', true),
            ],
          },
        }),
      ]),
    )
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'RefreshCrdInstances'))
      .toMatchObject({ data: { group: 'argoproj.io', kind: 'Application', version: 'v1alpha1' } })
    await emit(
      page,
      batch([
        system(2, {
          type: 'CrdInstancesUpdated',
          data: {
            group: 'argoproj.io',
            kind: 'Application',
            version: 'v1alpha1',
            namespace: null,
            instances: [app('guestbook', 'Synced', 'Healthy'), app('api', 'OutOfSync', 'Degraded')],
            failure: null,
          },
        }),
      ]),
    )
    const table = page.getByTestId('argo-table')
    await expect(table).toContainText('guestbook')
    await expect(table).toContainText('OutOfSync')
    await expect(page.getByText('Not installed: ApplicationSet')).toBeVisible()
    await expect(page.getByText('Permission denied listing: AppProject')).toBeVisible()
    // Read-only: no mutation controls.
    await expect(page.getByRole('button', { name: /delete|sync|apply/i })).toHaveCount(0)

    await page.getByRole('button', { name: 'Open Application api YAML' }).click()
    await expect(page.getByTestId('argo-panel')).toHaveCount(0)
  })
})

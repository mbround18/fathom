/** Batch D: HPA (namespaced) and validating/mutating webhooks (cluster) list + inspector, and the VPA unavailable state. */
import { expect, test } from '@playwright/test'

import { batch, list, nsRoute, read } from '../fixtures/builders'
import { HPA_DETAIL, HPA_ROW, MUTATING_DETAIL, MUTATING_ROW } from '../fixtures/batchD'
import { emit, expectRowCount, navigateTo, open, row, sent } from './helpers'

const HPA = 'config/horizontalpodautoscalers'
const MUTATING = 'config/mutatingwebhooks'

test.describe('Batch D config kinds', () => {
  test('HPA list opens an inspector with replicas, metrics and conditions', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Horizontal Pod Autoscalers', 'Config')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', HPA), {
          type: 'HorizontalPodAutoscalersUpdated',
          data: [HPA_ROW],
        }),
      ]),
    )
    await expectRowCount(page, 1, HPA)
    await row(page, 'default/web', HPA).click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SelectHorizontalPodAutoscaler'))
      .toEqual({
        type: 'SelectHorizontalPodAutoscaler',
        data: { namespace: 'default', name: 'web' },
      })
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HPA, 'default'), {
          type: 'HorizontalPodAutoscalerDetailsUpdated',
          data: HPA_DETAIL,
        }),
      ]),
    )
    const detail = page.locator('[data-testid="detail-sections"]').first()
    await expect(detail).toContainText('Desired replicas')
    await expect(detail.getByTestId('hpa-metric')).toContainText('current 85% / target 70%')
    await expect(detail).toContainText('ScalingActive=True')
  })

  test('mutating webhook inspector lists hooks with target and rules', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Mutating Webhook Configurations', 'Config')
    await emit(
      page,
      batch([
        list(
          1,
          { section: MUTATING, scope: 'Cluster', context: 'kind-a', namespace: null },
          { type: 'MutatingWebhookConfigurationsUpdated', data: [MUTATING_ROW] },
        ),
      ]),
    )
    await expectRowCount(page, 1, MUTATING)
    await row(page, MUTATING_ROW.name, MUTATING).click()
    await emit(
      page,
      batch([
        read(
          2,
          { section: MUTATING, scope: 'Cluster', context: 'kind-a', namespace: null },
          { type: 'MutatingWebhookConfigurationDetailsUpdated', data: MUTATING_DETAIL },
        ),
      ]),
    )
    const hook = page.locator('[data-testid="detail-sections"]').first().getByTestId('webhook-hook')
    await expect(hook).toContainText('inject.istio.io')
    await expect(hook).toContainText('failurePolicy Fail')
    await expect(hook).toContainText('service istio-system/istiod/inject')
    await expect(hook).toContainText('CREATE core/v1 pods')
  })

  test('Vertical Pod Autoscalers shows an honest unavailable state', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Vertical Pod Autoscalers', 'Config')
    await expect(page.getByTestId('unavailable-section')).toContainText(
      'not available in this build',
    )
  })
})

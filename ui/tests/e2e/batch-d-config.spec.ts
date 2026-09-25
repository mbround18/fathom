/** Batch D: HPA (namespaced) and validating/mutating webhooks (cluster) list + inspector, and VPA (list, not-installed state, delete). */
import { expect, test } from '@playwright/test'

import { batch, envelope, list, nsRoute, read } from '../fixtures/builders'
import {
  HPA_DETAIL,
  HPA_ROW,
  MUTATING_DETAIL,
  MUTATING_ROW,
  VPA_DETAIL,
  VPA_ROW,
} from '../fixtures/batchD'
import {
  check,
  clearSent,
  confirmDialog,
  emit,
  expectRowCount,
  navigateTo,
  open,
  requestIdOf,
  row,
  sent,
} from './helpers'

const HPA = 'config/horizontalpodautoscalers'
const VPA = 'config/verticalpodautoscalers'
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

  test('VPA list opens an inspector with target, mode and recommendations', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Vertical Pod Autoscalers', 'Config')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', VPA), { type: 'VerticalPodAutoscalersUpdated', data: [VPA_ROW] }),
      ]),
    )
    await expectRowCount(page, 1, VPA)
    await expect(row(page, 'default/web-vpa', VPA)).toContainText('Auto')
    await row(page, 'default/web-vpa', VPA).click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SelectVerticalPodAutoscaler'))
      .toEqual({
        type: 'SelectVerticalPodAutoscaler',
        data: { namespace: 'default', name: 'web-vpa' },
      })
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', VPA, 'default'), {
          type: 'VerticalPodAutoscalerDetailsUpdated',
          data: VPA_DETAIL,
        }),
      ]),
    )
    const detail = page.locator('[data-testid="detail-sections"]').first()
    await expect(detail).toContainText('Update mode')
    await expect(detail.getByTestId('vpa-recommendation')).toContainText(
      'lower cpu 25m / mem 100Mi · target cpu 50m / mem 200Mi · upper cpu 1 / mem 1Gi',
    )
    await expect(detail).toContainText('RecommendationProvided=True')
  })

  test('VPA shows a not-available state when the CRD is not installed', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Vertical Pod Autoscalers', 'Config')
    const id = await requestIdOf(page, 'RefreshVerticalPodAutoscalers')
    await emit(
      page,
      batch([
        envelope(
          3,
          nsRoute('kind-a', VPA),
          {
            type: 'System',
            data: {
              type: 'Notification',
              data: {
                level: 'Error',
                message: 'ApiError: the server could not find the requested resource (404)',
              },
            },
          },
          { type: 'Request', data: id },
        ),
      ]),
    )
    await expect(page.getByText('Vertical Pod Autoscalers not available')).toBeVisible()
  })

  test('VPA delete is confirmed and sends the exact command', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Vertical Pod Autoscalers', 'Config')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', VPA), { type: 'VerticalPodAutoscalersUpdated', data: [VPA_ROW] }),
      ]),
    )
    await check(page, 'default/web-vpa', VPA)
    await clearSent(page)
    await page.getByTestId('bulk-delete').click()
    await expect(confirmDialog(page).getByTestId('confirm-item')).toHaveCount(1)
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual([
        { type: 'DeleteVerticalPodAutoscaler', data: { namespace: 'default', name: 'web-vpa' } },
      ])
  })
})

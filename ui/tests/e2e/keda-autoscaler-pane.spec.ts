/**
 * 029-keda-autoscaler-pane: the Autoscalers tool pane lists ScaledObjects/ScaledJobs, shows an
 * honest not-installed state, and renders backend-provided trigger data + scale reasoning.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import type { KedaResourceDetails, KedaResourceSummary } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { emit, open, openToolPane, sentTypes } from './helpers'

async function openKeda(page: Page, info: TestInfo): Promise<void> {
  // The laptop class has neither a dock nor a tab strip, so no tool pane is reachable there.
  test.skip(info.project.name.endsWith('-laptop'), 'no tool-pane surface at laptop width')
  const stacked = info.project.name.endsWith('-phone') || info.project.name.endsWith('-tablet')
  if (stacked) {
    await openToolPane(page, 'Autoscalers')
  } else {
    await page.getByRole('tab', { name: 'Autoscalers' }).click()
  }
}

const QUEUE_WORKER: KedaResourceSummary = {
  kind: 'ScaledObject',
  name: 'queue-worker',
  namespace: 'default',
  target_kind: 'Deployment',
  target_name: 'queue-worker',
  target_found: true,
  current_replicas: 4,
  min_replicas: 1,
  max_replicas: 10,
  paused: false,
  ready: true,
  trigger_types: ['prometheus'],
  age: '3d',
}

function details(
  summary: KedaResourceSummary,
  state: 'MidRange' | 'AtFloor',
  message: string,
  live: boolean,
): KedaResourceDetails {
  return {
    summary,
    triggers: [
      {
        trigger_type: 'prometheus',
        metadata: { query: 'sum(rate(jobs[1m]))', threshold: '50' },
        auth_ref: null,
        active: null,
        current_value: live ? 73 : null,
        threshold: '50',
      },
    ],
    reasoning: { state, message },
  }
}

test.describe('KEDA autoscaler pane', () => {
  test('requests a refresh and shows the not-installed state', async ({ page }, info) => {
    await open(page)
    await openKeda(page, info)
    expect(await sentTypes(page)).toContain('RefreshKeda')
    await emit(
      page,
      batch([
        system(1, {
          type: 'KedaOverviewUpdated',
          data: { installed: false, scaled_objects: [], scaled_jobs: [] },
        }),
      ]),
    )
    await expect(page.getByText(/KEDA not detected/)).toBeVisible()
    await expect(page.getByTestId('keda-row')).toHaveCount(0)
  })

  test('lists a ScaledObject and renders trigger detail and reasoning', async ({ page }, info) => {
    await open(page)
    await openKeda(page, info)
    await emit(
      page,
      batch([
        system(1, {
          type: 'KedaOverviewUpdated',
          data: { installed: true, scaled_objects: [QUEUE_WORKER], scaled_jobs: [] },
        }),
      ]),
    )
    const row = page.getByTestId('keda-row')
    await expect(row).toHaveCount(1)
    await expect(row).toContainText('default/queue-worker')
    await expect(row).toContainText('4 replicas (min 1, max 10)')

    await row.click()
    await emit(
      page,
      batch([
        system(2, {
          type: 'KedaResourceDetailsUpdated',
          data: details(
            QUEUE_WORKER,
            'MidRange',
            'Scaled to 4 between the minimum (1) and maximum (10).',
            true,
          ),
        }),
      ]),
    )
    await expect(page.getByTestId('keda-trigger-0')).toContainText('Prometheus')
    await expect(page.getByTestId('keda-trigger-live')).toHaveText('current: 73 / threshold: 50')
    await expect(page.getByTestId('keda-reasoning')).toContainText('between the minimum (1)')

    // At the floor, with no live value: only type + threshold, no error.
    const atFloor = { ...QUEUE_WORKER, current_replicas: 1 }
    await emit(
      page,
      batch([
        system(3, {
          type: 'KedaResourceDetailsUpdated',
          data: details(
            atFloor,
            'AtFloor',
            'At floor (1) - no trigger currently reports active demand.',
            false,
          ),
        }),
      ]),
    )
    await expect(page.getByTestId('keda-reasoning')).toContainText('At floor (1)')
    await expect(page.getByTestId('keda-trigger-live')).toHaveCount(0)
    await expect(page.getByTestId('keda-trigger-0')).toContainText('threshold: 50')
  })
})

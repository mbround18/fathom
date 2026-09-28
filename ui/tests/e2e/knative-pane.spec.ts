/**
 * 028-knative-pane: the Knative tool pane renders inventory, traffic split, the autoscaler
 * "why", and an honest not-installed state. Data is injected through the fake backend.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import type { KnativeOverview } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { emit, open, openToolPane } from './helpers'

async function openKnative(page: Page, info: TestInfo): Promise<void> {
  // The laptop class has neither a dock nor a tab strip, so no tool pane is reachable there
  // (same limitation as the Topology pane).
  test.skip(info.project.name.endsWith('-laptop'), 'no tool-pane surface at laptop width')
  // Phone/tablet: the tool strip. Desktop and up: Integrations › Knative in the navigator (043).
  await openToolPane(page, 'Knative')
}

const OVERVIEW: KnativeOverview = {
  installed: true,
  services: [
    {
      name: 'hello',
      namespace: 'default',
      ready: 'Ready',
      latest_ready_revision: 'hello-00002',
      latest_created_revision: 'hello-00002',
    },
  ],
  revisions: [
    {
      name: 'hello-00001',
      namespace: 'default',
      service_name: 'hello',
      ready: 'Ready',
      actual_replicas: 0,
      desired_replicas: 0,
      scale_reason: { kind: 'ScaledToZero', data: { idle_since: null } },
      traffic_percent: 80,
    },
    {
      name: 'hello-00002',
      namespace: 'default',
      service_name: 'hello',
      ready: 'Ready',
      actual_replicas: 1,
      desired_replicas: 1,
      scale_reason: { kind: 'AtDesiredScale' },
      traffic_percent: 20,
    },
  ],
  routes: [
    {
      name: 'hello',
      namespace: 'default',
      service_name: 'hello',
      ready: 'Ready',
      url: null,
      traffic: [
        { revision_name: 'hello-00001', percent: 80, tag: null, revision_missing: false },
        { revision_name: 'hello-00002', percent: 20, tag: null, revision_missing: false },
      ],
    },
  ],
}

async function push(page: Page, overview: KnativeOverview): Promise<void> {
  await emit(page, batch([system(1, { type: 'KnativeOverviewUpdated', data: overview })]))
}

test.describe('Knative pane', () => {
  test('lists services, revisions and routes', async ({ page }, info) => {
    await open(page)
    await openKnative(page, info)
    await push(page, OVERVIEW)
    await expect(page.getByTestId('knative-service-row')).toHaveCount(1)
    await expect(page.getByTestId('knative-revision-row')).toHaveCount(2)
    await expect(page.getByTestId('knative-route-row')).toHaveCount(1)
  })

  test('shows the traffic split percentages', async ({ page }, info) => {
    await open(page)
    await openKnative(page, info)
    await push(page, OVERVIEW)
    const targets = page.getByTestId('knative-traffic-target')
    await expect(targets).toHaveText(['hello-00001 80%', 'hello-00002 20%'])
  })

  test('explains a scaled-to-zero revision', async ({ page }, info) => {
    await open(page)
    await openKnative(page, info)
    await push(page, OVERVIEW)
    await expect(page.getByTestId('knative-scale-reason').first()).toHaveText(
      'Scaled to zero: no traffic',
    )
  })

  test('shows the not-installed empty state', async ({ page }, info) => {
    await open(page)
    await openKnative(page, info)
    await push(page, { installed: false, services: [], revisions: [], routes: [] })
    await expect(page.getByText(/Knative Serving CRDs were not found/)).toBeVisible()
    await expect(page.getByTestId('knative-services')).toHaveCount(0)
    await expect(page.getByTestId('knative-traffic-graph')).toHaveCount(0)
  })
})

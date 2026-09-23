/**
 * US1 (007-topology-graph-view): "As a cluster operator, I want to see a live topology graph of
 * my cluster's namespaces, applications, workloads, and nodes, so I can understand the shape of
 * the cluster at a glance." Named `us1-topology-*` and all new hooks are `topology-*`-prefixed
 * (isolation from specs 008-010's own e2e suites, per the shared test-isolation convention).
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, list, clusterRoute, nsRoute } from '../fixtures/builders'
import { emit, open, sentTypes } from './helpers'

const APPLICATIONS = 'applications'
const NODES = 'nodes'

/** Reach the Topology tool pane the way each width class exposes it (FR-016). */
async function openTopology(page: Page, info: TestInfo): Promise<void> {
  const stacked = info.project.name.endsWith('-phone') || info.project.name.endsWith('-tablet')
  if (stacked) {
    await page.getByRole('navigation', { name: 'Views' }).getByRole('button', { name: 'Topology' }).click()
  } else {
    await page.getByRole('tab', { name: 'Topology' }).click()
  }
}

function canvasHost(page: Page) {
  return page.getByTestId('topology-canvas-host')
}

test.describe('US1 topology overview', () => {
  test('T041: opening Topology fetches the tiers it renders and shows the canvas', async ({
    page,
  }, info) => {
    await open(page)
    await openTopology(page, info)

    await expect(canvasHost(page)).toBeVisible()
    await expect(page.getByTestId('topology-canvas')).toBeVisible()

    // Mount-time refresh (TopologyPanel): every section the graph can draw without the operator
    // doing anything first (namespace/app/helm/node/workload tiers).
    await expect.poll(() => sentTypes(page)).toEqual(
      expect.arrayContaining([
        'RefreshApplications',
        'RefreshHelmReleases',
        'RefreshNodes',
        'RefreshDeployments',
        'RefreshStatefulSets',
        'RefreshDaemonSets',
      ]),
    )
  })

  test('T041: renders once application and node data arrive, with no error state', async ({
    page,
  }, info) => {
    await open(page)
    await openTopology(page, info)
    await expect(page.getByTestId('topology-canvas')).toBeVisible()

    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', APPLICATIONS), {
          type: 'ApplicationsUpdated',
          data: [
            {
              namespace: 'default',
              instance: 'web',
              name: 'web',
              managed_by: 'Helm',
              version: '1.0.0',
              creation_timestamp: '2024-01-15T10:00:00Z',
              ready_count: 3,
              total_count: 3,
            },
          ],
        }),
        list(2, clusterRoute('kind-a', NODES), {
          type: 'NodesUpdated',
          data: [
            {
              name: 'node-1',
              status: 'Ready',
              role: 'control-plane',
              pod_count: 12,
              pod_capacity: 110,
              cpu_allocatable_millicores: 4000,
              memory_allocatable_bytes: 8_000_000_000,
            },
          ],
        }),
      ]),
    )

    // No crash / no error boundary: the canvas host is still mounted and visible after data
    // lands, and the zoom readout (present as soon as the camera state initializes) is shown.
    await expect(canvasHost(page)).toBeVisible()
    await expect(canvasHost(page)).toContainText('%')
  })

  test('T041: the camera starts at 100% zoom', async ({ page }, info) => {
    await open(page)
    await openTopology(page, info)
    await expect(canvasHost(page)).toContainText('100%')
  })
})

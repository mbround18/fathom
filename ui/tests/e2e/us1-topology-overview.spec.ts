/**
 * US1 (007-topology-graph-view): "As a cluster operator, I want to see a live topology graph of
 * my cluster's namespaces, applications, workloads, and nodes, so I can understand the shape of
 * the cluster at a glance." Named `us1-topology-*` and all new hooks are `topology-*`-prefixed
 * (isolation from specs 008-010's own e2e suites, per the shared test-isolation convention).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, list, clusterRoute, nsRoute } from '../fixtures/builders'
import { emit, open, openTopologySection, sentTypes } from './helpers'

const APPLICATIONS = 'applications'
const NODES = 'nodes'

/** Open the whole-cluster Topology section from the navigator (043 US6, FR-030). */
async function openTopology(page: Page): Promise<void> {
  await openTopologySection(page)
}

function canvasHost(page: Page) {
  return page.getByTestId('topology-canvas-host')
}

test.describe('US1 topology overview', () => {
  test('T041: opening Topology fetches the tiers it renders and shows the canvas', async ({
    page,
  }) => {
    await open(page)
    await openTopology(page)

    await expect(canvasHost(page)).toBeVisible()
    await expect(page.getByTestId('topology-canvas')).toBeVisible()

    // Mount-time refresh (TopologyPanel): every section the graph can draw without the operator
    // doing anything first (namespace/app/helm/node/workload tiers).
    await expect
      .poll(() => sentTypes(page))
      .toEqual(
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
  }) => {
    await open(page)
    await openTopology(page)
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

  test('T041: the camera auto-fits the graph, capped at 200%, with a Fit control', async ({
    page,
  }) => {
    await open(page)
    await openTopology(page)
    // 041: the camera fits the laid-out graph (no longer a fixed 100%) and never zooms past
    // FIT_MAX_SCALE; the graph stays readable (not shrunk to dots) on every viewport.
    await expect(canvasHost(page).getByTestId('topology-fit')).toBeVisible()
    await expect
      .poll(async () => {
        const m = /(\d+)%/.exec((await canvasHost(page).textContent()) ?? '')
        return m ? Number(m[1]) : 0
      })
      .toBeGreaterThanOrEqual(30)
    const pct = Number(/(\d+)%/.exec((await canvasHost(page).textContent()) ?? '')?.[1])
    expect(pct).toBeLessThanOrEqual(200)
  })
})

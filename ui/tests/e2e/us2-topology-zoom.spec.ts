/**
 * US2 (007-topology-graph-view): "As a cluster operator, I want to zoom continuously from the
 * whole-cluster overview down to individual pods, so I can go from a high-level shape to a
 * specific problem without switching views." Named `us2-topology-*`, all new hooks
 * `topology-*`-prefixed (isolation from specs 008-010's own e2e suites).
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, list, read, clusterRoute, nsRoute } from '../fixtures/builders'
import { clearSent, emit, open, openToolPane, sentTypes } from './helpers'

const NAMESPACE = 'workloads-ns'
const POD_COUNT = 120

async function openTopology(page: Page, info: TestInfo): Promise<void> {
  // The laptop class has neither a dock nor a tab strip, so no tool pane is reachable there.
  test.skip(info.project.name.endsWith('-laptop'), 'no tool-pane surface at laptop width')
  const stacked = info.project.name.endsWith('-phone') || info.project.name.endsWith('-tablet')
  if (stacked) {
    await openToolPane(page, 'Topology')
  } else {
    await page.getByRole('tab', { name: 'Topology' }).click()
  }
}

function canvasHost(page: Page) {
  return page.getByTestId('topology-canvas-host')
}

async function seedOverview(page: Page): Promise<void> {
  // The panel's mount-time refresh is answered by the fake on a later macrotask with the default
  // scenario's data; seeding before that reply lands gets the seed overwritten (seen at ultra-plus,
  // where the busier shell delays the mount). Wait for the refresh, then let its reply flush.
  await expect.poll(() => sentTypes(page)).toContain('RefreshDeployments')
  await page.evaluate(() => new Promise<void>((r) => setTimeout(r, 50)))
  await emit(
    page,
    batch([
      list(1, clusterRoute('kind-a', 'namespaces'), {
        type: 'NamespacesUpdated',
        data: [NAMESPACE],
      }),
      list(2, nsRoute('kind-a', 'workloads/deployments'), {
        type: 'DeploymentsUpdated',
        data: [
          { name: 'web', namespace: NAMESPACE, desired: 3, ready: 3, updated: 3, available: 3 },
        ],
      }),
    ]),
  )
}

async function zoomCanvas(page: Page, deltaY: number): Promise<void> {
  const canvas = page.getByTestId('topology-canvas')
  const box = await canvas.boundingBox()
  if (!box) throw new Error('canvas not laid out yet')
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.wheel(0, deltaY)
}

/**
 * Zoom in until the workload focus fires `SelectDeployment`. A single wheel step can land before
 * the previous camera update is applied on very wide canvases under parallel load, so retry.
 */
async function zoomUntilSelectDeployment(page: Page): Promise<void> {
  await expect(async () => {
    await zoomCanvas(page, -900)
    expect(await sentTypes(page)).toContain('SelectDeployment')
  }).toPass({ timeout: 15_000 })
}

test.describe('US2 topology zoom', () => {
  test('T034: overview -> workload tier appears once zoomed into a namespace', async ({
    page,
  }, info) => {
    await open(page)
    await openTopology(page, info)
    await seedOverview(page)
    await expect(canvasHost(page)).toContainText('100%')

    // At the overview scale, the workload tier isn't fetched yet -- only the tiers T023 seeds
    // on mount.
    await clearSent(page)

    // Zoom in past NAMESPACE_FOCUS_SCALE (2.2): the workload tier becomes visible for every
    // namespace currently on screen (viewport.ts) -- the Deployment already in `lists` renders
    // without any new fetch (data-model.md: workload-tier rows come from the list already loaded
    // for the Deployments section).
    await zoomCanvas(page, -900)
    await expect(canvasHost(page)).not.toContainText('100%')

    // Zoom in further past WORKLOAD_FOCUS_SCALE (5): the pod tier for the now-focused Deployment
    // is fetched via the workload descriptor's own `select` command (T032).
    await zoomUntilSelectDeployment(page)
  })

  test('T034/FR-012: a large pod count stays responsive to a subsequent click', async ({
    page,
  }, info) => {
    await open(page)
    await openTopology(page, info)
    await seedOverview(page)

    await zoomCanvas(page, -900) // namespace focus
    await zoomUntilSelectDeployment(page) // workload focus -> triggers SelectDeployment

    const pods = Array.from({ length: POD_COUNT }, (_, i) => ({
      name: `web-${String(i)}`,
      namespace: NAMESPACE,
      phase: 'Running',
      node: 'node-1',
      ready: 1,
      total: 1,
      restarts: 0,
    }))
    await emit(
      page,
      batch([
        read(3, nsRoute('kind-a', 'workloads/deployments', NAMESPACE), {
          type: 'DeploymentDetailsUpdated',
          data: {
            namespace: NAMESPACE,
            name: 'web',
            creation_timestamp: '2024-01-15T10:00:00Z',
            labels: {},
            annotations: {},
            containers: ['app'],
            ports: [],
            pod_selector: 'app=web',
            pods,
            raw_json: '{}',
          },
        }),
      ]),
    )

    // 120 pod nodes laid out and drawn every frame: the canvas must still respond to input (no
    // hang/crash) and a click still resolves to a hit-test result -- the camera's zoom readout
    // keeps updating after the click, proving the input handlers are still live.
    const canvas = page.getByTestId('topology-canvas')
    const before = await canvasHost(page).textContent()
    await canvas.click({ position: { x: 5, y: 5 } }) // empty space: click-to-inspect is a no-op
    // Zoom out: the retrying zoom-in above may already sit at the max scale, where zooming in
    // further would not change the readout.
    await zoomCanvas(page, 100)
    await expect(canvasHost(page)).toBeVisible()
    await expect.poll(async () => canvasHost(page).textContent()).not.toBe(before)
  })

  test('T034: zooming back out restores the overview in the same pane', async ({ page }, info) => {
    await open(page)
    await openTopology(page, info)
    await seedOverview(page)

    await zoomCanvas(page, -900)
    await expect(canvasHost(page)).not.toContainText('100%')

    await zoomCanvas(page, 900)
    await expect(canvasHost(page)).toContainText('100%')
    await expect(page.getByTestId('topology-canvas')).toBeVisible()
  })
})

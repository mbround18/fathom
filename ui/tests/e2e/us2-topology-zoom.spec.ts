/**
 * US2 (007-topology-graph-view): "As a cluster operator, I want to zoom continuously from the
 * whole-cluster overview down to individual pods, so I can go from a high-level shape to a
 * specific problem without switching views." Named `us2-topology-*`, all new hooks
 * `topology-*`-prefixed (isolation from specs 008-010's own e2e suites).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, list, read, clusterRoute, nsRoute } from '../fixtures/builders'
import {
  clearSent,
  emit,
  open,
  openTopologySection,
  sentTypes,
  settledTopologyScale,
  topologyScale,
} from './helpers'

const NAMESPACE = 'workloads-ns'
const POD_COUNT = 120

/** Open the whole-cluster Topology section from the navigator (043 US6, FR-030). */
async function openTopology(page: Page): Promise<void> {
  await openTopologySection(page)
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
 * A canvas-relative point with nothing drawn around it, read from the settled canvas. The pod tier
 * is laid out after the details arrive and the camera's scale depends on how many wheel steps the
 * zoom took, so no hard-coded corner is reliably empty: at high scale the pods fill the canvas
 * (a click at (5, 5) opened a pod's inspector and unmounted the canvas). Waits until consecutive
 * frames are identical (layout and draw have settled), then searches a grid for a fully
 * transparent 11x11 patch. A transparent pixel can never be a hit-test hit.
 */
async function emptyCanvasPoint(page: Page): Promise<{ x: number; y: number }> {
  const canvas = page.getByTestId('topology-canvas')
  let found: { x: number; y: number } | null = null
  await expect(async () => {
    found = await canvas.evaluate(async (el: HTMLCanvasElement) => {
      let prev = el.toDataURL()
      for (let i = 0; i < 3; i++) {
        await new Promise<void>((r) => requestAnimationFrame(() => r()))
        await new Promise((r) => setTimeout(r, 100))
        const next = el.toDataURL()
        if (next !== prev) return null // still drawing
        prev = next
      }
      const g = el.getContext('2d')
      if (!g) return null
      const scale = el.width / el.clientWidth
      const half = 5
      for (let y = 8; y < el.clientHeight - 8; y += 16) {
        for (let x = 8; x < el.clientWidth - 8; x += 16) {
          const data = g.getImageData(
            (x - half) * scale,
            (y - half) * scale,
            (2 * half + 1) * scale,
            (2 * half + 1) * scale,
          ).data
          let empty = true
          for (let i = 3; i < data.length; i += 4) {
            if (data[i] !== 0) {
              empty = false
              break
            }
          }
          if (empty) return { x, y }
        }
      }
      return null
    })
    expect(found).not.toBeNull()
  }).toPass({ timeout: 15_000 })
  return found as unknown as { x: number; y: number }
}

/**
 * Zoom in until the workload focus fires `SelectDeployment`. Each wheel step is followed by a wait
 * for the zoom readout to change, so a step is never re-sent while the previous one is still
 * queued behind a busy main thread. Re-sending blindly compounded the steps and pushed the camera
 * to the max scale, where pod nodes fill the whole canvas and no point is empty space.
 */
async function zoomUntilSelectDeployment(page: Page): Promise<void> {
  const host = canvasHost(page)
  await expect(async () => {
    const before = await host.textContent()
    await zoomCanvas(page, -900)
    await expect.poll(async () => host.textContent(), { timeout: 3_000 }).not.toBe(before)
    await expect.poll(() => sentTypes(page), { timeout: 2_000 }).toContain('SelectDeployment')
  }).toPass({ timeout: 15_000 })
}

test.describe('US2 topology zoom', () => {
  test('T034: overview -> workload tier appears once zoomed into a namespace', async ({ page }) => {
    await open(page)
    await openTopology(page)
    await seedOverview(page)
    const fit = await settledTopologyScale(page)

    // At the overview scale, the workload tier isn't fetched yet -- only the tiers T023 seeds
    // on mount.
    await clearSent(page)

    // Zoom in past NAMESPACE_FOCUS_SCALE (2.2): the workload tier becomes visible for every
    // namespace currently on screen (viewport.ts) -- the Deployment already in `lists` renders
    // without any new fetch (data-model.md: workload-tier rows come from the list already loaded
    // for the Deployments section).
    await zoomCanvas(page, -900)
    await expect.poll(() => topologyScale(page)).toBeGreaterThan(fit)

    // Zoom in further past WORKLOAD_FOCUS_SCALE (5): the pod tier for the now-focused Deployment
    // is fetched via the workload descriptor's own `select` command (T032).
    await zoomUntilSelectDeployment(page)
  })

  test('T034/FR-012: a large pod count stays responsive to a subsequent click', async ({
    page,
  }) => {
    await open(page)
    await openTopology(page)
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
    // Empty space: click-to-inspect is a no-op.
    await canvas.click({ position: await emptyCanvasPoint(page) })
    // Zoom out: the retrying zoom-in above may already sit at the max scale, where zooming in
    // further would not change the readout.
    await zoomCanvas(page, 100)
    await expect(canvasHost(page)).toBeVisible()
    await expect.poll(async () => canvasHost(page).textContent()).not.toBe(before)
  })

  test('T034: zooming back out restores the overview in the same pane', async ({ page }) => {
    await open(page)
    await openTopology(page)
    await seedOverview(page)
    const fit = await settledTopologyScale(page)

    await zoomCanvas(page, -900)
    await expect.poll(() => topologyScale(page)).toBeGreaterThan(fit)

    // Wheel zoom is multiplicative (exp(-deltaY / 1000)), so the same wheel back lands on the fit.
    await zoomCanvas(page, 900)
    await expect.poll(() => topologyScale(page)).toBe(fit)
    await expect(page.getByTestId('topology-canvas')).toBeVisible()
  })
})

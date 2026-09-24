/**
 * US3 (007-topology-graph-view): "As a cluster operator, I want to click any node in the graph
 * to open the same inspector I'd get from the resource list, so the graph is a real navigation
 * surface, not just a picture." Named `us3-topology-*`, all new hooks `topology-*`-prefixed
 * (isolation from specs 008-010's own e2e suites).
 */
import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test'

import { batch, list, clusterRoute } from '../fixtures/builders'
import { emit, inspector, open, openToolPane } from './helpers'

const NODES = 'nodes'

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

/**
 * A single node with no edges settles at (or very near) the layout's `forceCenter` target (the
 * only force acting on it), which is also where the default identity camera points. A small grid
 * around the canvas's own center absorbs any residual simulation slack without needing to read
 * exact pixel positions out of the page.
 */
async function clickNear(page: Page, canvas: Locator): Promise<void> {
  const box = await canvas.boundingBox()
  if (!box) throw new Error('canvas not laid out yet')
  const cx = box.x + box.width / 2
  const cy = box.y + box.height / 2
  const offsets = [0, 8, -8, 16, -16, 24, -24]
  for (const dx of offsets) {
    for (const dy of offsets) {
      await page.mouse.click(cx + dx, cy + dy)
      if (await inspector(page).isVisible()) return
    }
  }
}

async function seedSoloNode(page: Page): Promise<void> {
  await emit(
    page,
    batch([
      list(1, clusterRoute('kind-a', 'namespaces'), { type: 'NamespacesUpdated', data: [] }),
      list(2, clusterRoute('kind-a', NODES), {
        type: 'NodesUpdated',
        data: [
          {
            name: 'solo-node',
            status: 'Ready',
            role: 'control-plane',
            pod_count: 4,
            pod_capacity: 110,
            cpu_allocatable_millicores: 4000,
            memory_allocatable_bytes: 8_000_000_000,
          },
        ],
      }),
    ]),
  )
}

test.describe('US3 topology inspector navigation', () => {
  test('T043: clicking a graph node opens its inspector', async ({ page }, info) => {
    await open(page)
    await openTopology(page, info)
    await seedSoloNode(page)

    const canvas = page.getByTestId('topology-canvas')
    await expect(canvas).toBeVisible()

    // The layout simulation runs in an effect after the graph settles; retry the click grid
    // until it lands (the node's on-screen position converges near canvas center once laid
    // out, just not necessarily on the very first paint).
    await expect(async () => {
      await clickNear(page, canvas)
      await expect(inspector(page)).toBeVisible({ timeout: 300 })
    }).toPass({ timeout: 8000 })

    await expect(inspector(page)).toContainText('solo-node')
  })

  test('T042: the camera viewport persists across an inspector round-trip', async ({
    page,
  }, info) => {
    await open(page)
    await openTopology(page, info)
    await seedSoloNode(page)

    const canvas = page.getByTestId('topology-canvas')
    const host = page.getByTestId('topology-canvas-host')
    await expect(canvas).toBeVisible()

    // Zoom in (wheel) so the camera differs from the default before navigating away.
    const box = await canvas.boundingBox()
    if (!box) throw new Error('canvas not laid out yet')
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.wheel(0, -400)
    await expect(host).not.toContainText('100%')
    const zoomedLabel = await host.textContent()

    // Open the inspector, then back out of it (US3's "back-stack", not a fresh navigation).
    await expect(async () => {
      await clickNear(page, canvas)
      await expect(inspector(page)).toContainText('solo-node', { timeout: 300 })
    }).toPass({ timeout: 8000 })

    // Escape clears the inspector's target (desktop: the pane's slot goes back to "pick a
    // target"; phone/tablet: the whole pane pops off the stack) — either way, the topology
    // canvas is back in view afterward with its camera untouched.
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('topology-canvas-host')).toBeVisible()
    await expect(page.getByTestId('topology-canvas-host')).toContainText(zoomedLabel ?? '')
  })
})

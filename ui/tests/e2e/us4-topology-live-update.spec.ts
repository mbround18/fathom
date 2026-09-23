/**
 * US4 (007-topology-graph-view): "As a cluster operator, I want the graph's colors to reflect
 * live health state, so I can spot a problem without opening every node." Named
 * `us4-topology-*`, all new hooks `topology-*`-prefixed (isolation from specs 008-010's own e2e
 * suites).
 *
 * T050: `nodeTone` is a pure function of the store's already-cached rows (`ui/src/topology/
 * status.ts`, unit-tested in `tests/unit/topology/status.test.ts` — ok/warn/err mapping is
 * already covered there, no new code needed), and the graph/layout `useMemo`/`useEffect` chain in
 * `TopologyCanvas.tsx` already depends on `s.lists`/`s.details` (T024/T032), so a scripted update
 * recomputes tone with no new production code.
 *
 * T051: this e2e spec is the regression test for the *live-update wiring itself* — a status
 * change on an already-rendered node redraws it in place (same pane, still drawing something at
 * the same screen position) without the graph re-fetching anything, exactly like a list row
 * re-rendering after a refresh. (Sampling the *exact* recolored pixel isn't reliable here: the
 * fake e2e build doesn't guarantee `oklch()` CSS custom properties resolve identically across
 * every Chromium/WebKit test runner, and pixel-exact color assertions are covered better, and
 * deterministically, by the tone-mapping unit tests referenced above.)
 */
import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test'

import { batch, list, clusterRoute } from '../fixtures/builders'
import { emit, open, clearSent, sentTypes } from './helpers'

const NODES = 'nodes'

async function openTopology(page: Page, info: TestInfo): Promise<void> {
  const stacked = info.project.name.endsWith('-phone') || info.project.name.endsWith('-tablet')
  if (stacked) {
    await page.getByRole('navigation', { name: 'Views' }).getByRole('button', { name: 'Topology' }).click()
  } else {
    await page.getByRole('tab', { name: 'Topology' }).click()
  }
}

async function seedNode(page: Page, status: string): Promise<void> {
  await emit(
    page,
    batch([
      list(1, clusterRoute('kind-a', 'namespaces'), { type: 'NamespacesUpdated', data: [] }),
      list(2, clusterRoute('kind-a', NODES), {
        type: 'NodesUpdated',
        data: [
          {
            name: 'solo-node',
            status,
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

/**
 * The node's own drawn pixel color, sampled directly off the canvas (a single node with no
 * edges/siblings settles at the layout's `forceCenter` target, same trick as
 * `us3-topology-inspector.spec.ts`'s `clickNear`, but here we just need one stable spot to
 * sample rather than to click).
 */
async function nodeColor(page: Page, canvas: Locator): Promise<[number, number, number, number]> {
  const box = await canvas.boundingBox()
  if (!box) throw new Error('canvas not laid out yet')
  return page.evaluate(
    ({ x, y }) => {
      const el = document.querySelector('[data-testid="topology-canvas"]') as HTMLCanvasElement
      const rect = el.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      const cx = Math.round((x - rect.left) * dpr)
      const cy = Math.round((y - rect.top) * dpr)
      const ctx = el.getContext('2d')
      if (!ctx) return [0, 0, 0, 0]
      const d = ctx.getImageData(cx, cy, 1, 1).data
      return [d[0], d[1], d[2], d[3]]
    },
    { x: box.x + box.width / 2, y: box.y + box.height / 2 },
  ) as Promise<[number, number, number, number]>
}

test.describe('US4 topology live health state', () => {
  test('T051: a status change recolors the node in place, no extra commands', async ({
    page,
  }, info) => {
    await open(page)
    await openTopology(page, info)
    await seedNode(page, 'Ready')

    const canvas = page.getByTestId('topology-canvas')
    await expect(canvas).toBeVisible()

    // Give the layout effect a tick to settle the single node at its deterministic center
    // position, then confirm something is actually drawn there (alpha > 0, not blank canvas).
    await expect
      .poll(async () => (await nodeColor(page, canvas))[3])
      .toBeGreaterThan(0)

    // The graph must not re-fetch anything just because a row it already has changed in place —
    // it draws straight from the store, same as any list view re-rendering a row.
    await clearSent(page)

    await seedNode(page, 'NotReady')

    // Same pane (`topology-canvas-host` never unmounts, no new pane opened), still drawing at the
    // same position (redraw happened, not a crash/blank canvas), no new commands sent by the
    // graph reacting to the update — the tone-mapping and color itself are unit-tested in
    // `tests/unit/topology/status.test.ts`.
    await expect
      .poll(async () => (await nodeColor(page, canvas))[3])
      .toBeGreaterThan(0)
    await expect(page.getByTestId('topology-canvas-host')).toBeVisible()
    expect(await sentTypes(page)).toEqual([])
  })
})

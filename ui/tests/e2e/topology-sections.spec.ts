/**
 * 043 US6 (T060): topology lives on the resource and in its own section, not the bottom dock.
 *
 *  - Cluster › Topology opens the whole-cluster graph (and the daemon's view while it is up).
 *  - A Deployment's inspector has a Topology tab: it asks the related-resources resolver and
 *    shows the ReplicaSet and Pods; "Follow" on a Pod re-roots the inspector there, with Back,
 *    and the Topology tab stays selected. A ConfigMap has no Topology tab.
 *  - The phone and tablet Topology views and the laptop Tools menu still open the whole view
 *    (FR-033), and a saved layout pinned to the old `topology` dock tool opens the section.
 *
 * Expanding nodes in place and click-through inside the graph are feedback #23, not tested here.
 */
import { expect, test, type Page } from '@playwright/test'

import type { UiConfig } from '../../src/protocol/types'
import { PODS_500 } from '../fixtures/batches'
import { BATCH_C_LIST_EVENTS } from '../fixtures/batchCKinds'
import { batch, list, nsRoute, read, system } from '../fixtures/builders'
import { daemonStatus, sampleTopology, topologyBatch } from '../fixtures/daemon'
import {
  clearSent,
  emit,
  inspector,
  navigateTo,
  open,
  openToolPane,
  openTopologySection,
  row,
  sent,
  sentTypes,
} from './helpers'
import { widthClass } from './layout-helpers'

const DEPLOYMENTS = 'workloads/deployments'
const NAMESPACE = 'payments'
const DEPLOYMENT = 'web'
const REPLICASET = 'web-7f9c6d5b4'
const POD = PODS_500.find((p) => p.namespace === NAMESPACE && p.name.startsWith(`${DEPLOYMENT}-`))
if (!POD) throw new Error('fixture: no payments/web pod')

const topologyTab = (page: Page) => inspector(page).getByTestId('topology-tab')
const neighbours = (page: Page) => inspector(page).getByTestId('inspector-topology')

async function openDeployment(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Deployments', 'Workloads')
  // One row, so it is on screen at every width: let the scenario's own reply land first, or it
  // overwrites the seed (the same wait us2-topology-zoom uses).
  await expect.poll(() => sentTypes(page)).toContain('RefreshDeployments')
  await page.evaluate(() => new Promise<void>((r) => setTimeout(r, 50)))
  await emit(
    page,
    batch([
      list(2, nsRoute('kind-a', DEPLOYMENTS), {
        type: 'DeploymentsUpdated',
        data: [
          {
            name: DEPLOYMENT,
            namespace: NAMESPACE,
            desired: 2,
            ready: 2,
            updated: 2,
            available: 2,
          },
        ],
      }),
    ]),
  )
  const target = row(page, `${NAMESPACE}/${DEPLOYMENT}`, DEPLOYMENTS)
  await expect(target).toBeVisible()
  await target.click()
  await expect(inspector(page)).toContainText(DEPLOYMENT)
}

async function answerRelated(
  page: Page,
  seq: number,
  root: { section: string; kind: string; name: string },
  related: unknown[],
): Promise<void> {
  await emit(
    page,
    batch([
      read(seq, nsRoute('kind-a', root.section, NAMESPACE), {
        type: 'RelatedResourcesUpdated',
        data: { ...root, namespace: NAMESPACE, related },
      } as never),
    ]),
  )
}

const DEPLOYMENT_RELATED = [
  {
    section: 'workloads/replicasets',
    kind: 'ReplicaSet',
    namespace: NAMESPACE,
    name: REPLICASET,
    relation: 'owns',
    status_hint: null,
  },
  {
    section: 'workloads/pods',
    kind: 'Pod',
    namespace: NAMESPACE,
    name: POD.name,
    relation: 'selects',
    status_hint: 'Running',
  },
]

test.describe('043 US6 topology sections', () => {
  test('Cluster › Topology opens the whole-cluster graph', async ({ page }) => {
    await open(page)
    await openTopologySection(page)
    await expect(page.getByTestId('topology-panel')).toBeVisible()
    await expect(page.getByTestId('topology-canvas-host')).toBeVisible()
    await expect(page.locator('[data-slot="pane"][data-section="topology"]').first()).toBeVisible()
  })

  test('Cluster › Topology shows the daemon view while the daemon is available', async ({
    page,
  }) => {
    await open(page)
    await emit(page, daemonStatus('available'))
    await emit(page, topologyBatch(1, sampleTopology()))
    await openTopologySection(page)
    await expect(page.getByTestId('daemon-topology')).toBeVisible()
  })

  test("a Deployment's Topology tab shows its ReplicaSet and Pods", async ({ page }) => {
    await openDeployment(page)
    await clearSent(page)
    await topologyTab(page).click()

    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'RefreshRelatedResources'))
      .toMatchObject({
        data: { section: DEPLOYMENTS, kind: 'Deployment', namespace: NAMESPACE, name: DEPLOYMENT },
      })
    await answerRelated(
      page,
      500,
      { section: DEPLOYMENTS, kind: 'Deployment', name: DEPLOYMENT },
      DEPLOYMENT_RELATED,
    )

    const view = neighbours(page)
    await expect(view.getByTestId('topology-root')).toContainText(DEPLOYMENT)
    await expect(view.getByRole('region', { name: /Downstream/ })).toContainText(REPLICASET)
    await expect(view.getByRole('region', { name: /Downstream/ })).toContainText(POD.name)
    await expect(
      view.getByRole('listitem', { name: `ReplicaSet ${NAMESPACE}/${REPLICASET}` }),
    ).toBeVisible()
  })

  /**
   * 044 (FR-005): at wide and above a Follow opens the Pod in the next cascade column instead of
   * re-rooting the column it was clicked in, so there is no back-stack to walk — the columns are
   * the trail. The next test covers that; this one keeps the pre-044 behaviour where it still
   * applies, below wide.
   */
  test('Follow on a Pod opens it, with Back, and keeps the Topology tab', async ({ page }) => {
    await openDeployment(page)
    test.skip(
      ['wide', 'ultra'].includes((await widthClass(page)) ?? ''),
      'the cascade opens the next column instead (FR-005)',
    )
    await topologyTab(page).click()
    await expect.poll(() => sentTypes(page)).toContain('RefreshRelatedResources')
    await answerRelated(
      page,
      500,
      { section: DEPLOYMENTS, kind: 'Deployment', name: DEPLOYMENT },
      DEPLOYMENT_RELATED,
    )
    await clearSent(page)
    await neighbours(page)
      .getByRole('button', { name: `Follow Pod ${NAMESPACE}/${POD.name}` })
      .click()

    await expect
      .poll(async () =>
        (await sent(page)).find(
          (c) => c.type === 'RefreshRelatedResources' && c.data.kind === 'Pod',
        ),
      )
      .toMatchObject({ data: { namespace: NAMESPACE, name: POD.name } })
    await expect(topologyTab(page)).toHaveAttribute('aria-selected', 'true')
    await answerRelated(page, 501, { section: 'workloads/pods', kind: 'Pod', name: POD.name }, [
      {
        section: DEPLOYMENTS,
        kind: 'ReplicaSet',
        namespace: NAMESPACE,
        name: REPLICASET,
        relation: 'owned-by',
        status_hint: null,
      },
    ])
    await expect(neighbours(page).getByTestId('topology-root')).toContainText(POD.name)
    await expect(neighbours(page).getByRole('region', { name: /Upstream/ })).toContainText(
      REPLICASET,
    )

    const breadcrumb = inspector(page).getByRole('navigation', { name: 'Breadcrumb' })
    await expect(breadcrumb).toContainText(DEPLOYMENT)
    await breadcrumb.getByRole('button', { name: '← Back' }).click()
    await expect(topologyTab(page)).toHaveAttribute('aria-selected', 'true')
    await expect(neighbours(page).getByTestId('topology-root')).toContainText(DEPLOYMENT)
  })

  test('at wide, Follow on a Pod opens the next cascade column (044 FR-005)', async ({ page }) => {
    await openDeployment(page)
    test.skip(
      !['wide', 'ultra'].includes((await widthClass(page)) ?? ''),
      'the cascade starts at wide (FR-009)',
    )
    await topologyTab(page).click()
    await expect.poll(() => sentTypes(page)).toContain('RefreshRelatedResources')
    await answerRelated(
      page,
      500,
      { section: DEPLOYMENTS, kind: 'Deployment', name: DEPLOYMENT },
      DEPLOYMENT_RELATED,
    )
    await neighbours(page)
      .getByRole('button', { name: `Follow Pod ${NAMESPACE}/${POD.name}` })
      .click()

    // Column 1 keeps the Deployment, column 2 is the Pod it led to.
    const column = (n: number) => page.locator(`[data-testid="cascade-step"][data-step="${n}"]`)
    await expect(column(1)).toHaveAccessibleName(DEPLOYMENT)
    await expect(column(2)).toHaveAccessibleName(POD.name)
  })

  test('a ConfigMap has no Topology tab', async ({ page }) => {
    const section = 'config/configmaps'
    await open(page)
    await navigateTo(page, 'ConfigMaps', 'Config')
    const configMaps = BATCH_C_LIST_EVENTS[section]
    if (!configMaps) throw new Error('fixture: no ConfigMaps list')
    await emit(page, batch([list(1, nsRoute('kind-a', section), configMaps)]))
    await row(page, 'default/app-config', section).click()
    await expect(inspector(page).getByRole('tab', { name: 'Details' })).toBeVisible()
    await expect(inspector(page).getByRole('tab', { name: 'YAML' })).toBeVisible()
    await expect(topologyTab(page)).toHaveCount(0)
  })

  test('the phone and tablet Topology views and the Tools menu still open it (FR-033)', async ({
    page,
  }) => {
    await open(page)
    const cls = (await widthClass(page)) ?? ''
    test.skip(!['phone', 'tablet', 'laptop'].includes(cls), 'no Topology tool entry at this width')
    await openToolPane(page, 'Topology')
    await expect(page.getByTestId('topology-panel')).toBeVisible()
    await expect(page.getByTestId('topology-canvas-host')).toBeVisible()
  })

  /**
   * 044 (FR-011): at wide and above every slot past the section list is a cascade column, so a
   * saved `tool:topology` slot cannot survive there (the test below covers what happens instead).
   * Below wide a saved tool slot still keeps its place, and 043's mapping still opens the section.
   */
  test('a saved layout pinned to the old topology dock tool opens the section', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 900 })
    await open(page)
    test.skip((await widthClass(page)) !== 'desktop', 'desktop only')
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(e.message))
    const saved: UiConfig = {
      schema: 1,
      theme: { mode: 'dark', accent: 'cyan', effects: 'on', glitch: false },
      density: {},
      layouts: {
        desktop: {
          slots: [
            { kind: 'nav', sizePct: 16 },
            { kind: 'list', sizePct: 44 },
            { kind: 'tool', tool: 'topology', sizePct: 40 },
          ],
          dock: { open: true, heightPct: 30, tabs: ['logs', 'events'] },
        },
      },
    }
    await emit(
      page,
      batch([
        system(2, {
          type: 'UiConfigLoaded',
          data: { ui: saved, expanded_groups: ['Workloads'], mcp_stdio_enabled: false },
        }),
      ]),
    )
    const slot = page.locator('[data-slot="pane"][data-slot-index="2"]')
    await expect(slot).toHaveAttribute('data-section', 'topology')
    await expect(slot.getByTestId('topology-panel')).toBeVisible()
    // The other list slot keeps its own section.
    await expect(page.locator('[data-slot="pane"][data-slot-index="1"]')).not.toHaveAttribute(
      'data-section',
      'topology',
    )
    expect(errors).toEqual([])
  })

  test('at wide the same saved tool slot becomes an empty cascade column (044 FR-011)', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1920, height: 1080 })
    await open(page)
    test.skip((await widthClass(page)) !== 'wide', 'wide only')
    const errors: string[] = []
    page.on('pageerror', (e) => errors.push(e.message))
    const saved: UiConfig = {
      schema: 1,
      theme: { mode: 'dark', accent: 'cyan', effects: 'on', glitch: false },
      density: {},
      layouts: {
        wide: {
          slots: [
            { kind: 'nav', sizePct: 10 },
            { kind: 'list', sizePct: 40 },
            { kind: 'tool', tool: 'topology', sizePct: 25 },
            { kind: 'inspector', sizePct: 25 },
          ],
          dock: { open: true, heightPct: 30, tabs: ['logs', 'events'] },
        },
      },
    }
    await emit(
      page,
      batch([
        system(2, {
          type: 'UiConfigLoaded',
          data: { ui: saved, expanded_groups: ['Workloads'], mcp_stdio_enabled: false },
        }),
      ]),
    )
    // The row is the cascade, so the tool slot is a drill-in column waiting to be filled; the
    // Topology section is still reachable from the navigator, which the tests above cover.
    const slot = page.locator('[data-slot="pane"][data-slot-index="2"]')
    await expect(slot).toHaveAttribute('data-pane-kind', 'inspector')
    await expect(slot.getByTestId('cascade-empty')).toBeVisible()
    await expect(page.getByTestId('topology-panel')).toHaveCount(0)
    expect(errors).toEqual([])
  })
})

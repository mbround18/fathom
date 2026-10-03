/**
 * 048-docker-management US4: containers and clusters are one world.
 *
 * The five acceptance scenarios of the story. A kind node is an ordinary container on this
 * daemon, so the containers list and the clusters list are the same objects seen from two sides —
 * and an operator who does not know that is one mis-click from killing the cluster they are
 * working in. That is why this is a safety story and not a convenience one.
 *
 * Two claims here are worth reading twice.
 *
 * The warning is two sentences, not one (FR-031). "This breaks a cluster" and "you are connected
 * to that cluster right now" are different facts: the first is true wherever the operator is
 * standing, and collapsing them into one line would make the second unsayable. So the same stop
 * is previewed twice here, from two contexts, and only one of them says the second sentence.
 *
 * And cluster identity comes from the label and from nothing else. A busybox container *named*
 * `kind-control-plane` carrying no cluster label is claimed for no cluster, because a warning
 * derived from a naming pattern would attach itself to unrelated containers — and a warning that
 * is sometimes about nothing is one the operator learns to click past.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, stream } from '../fixtures/builders'
import type { KindClusterSummary } from '../../src/protocol/types'
import { CLUSTER_CONTAINERS } from '../fixtures/containers'
import { emit, navigateTo, open, sentTypes } from './helpers'

/**
 * The Containers group holds a section of its own name, so the click is scoped to the group's
 * own section list — which is what `navigateTo` does, and it also waits for the phone drawer to
 * finish closing before the next navigation touches it.
 */
async function openContainers(page: Page, label = 'Containers'): Promise<void> {
  await navigateTo(page, label, 'Containers')
}

/** The containers scene with the inventory on screen. */
async function scene(page: Page, scenario: Parameters<typeof open>[1]): Promise<void> {
  await open(page, scenario)
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
  await openContainers(page)
  await expect(page.getByTestId('container-group-list')).toBeVisible()
}

/** The node container of the cluster `kind-a` — the context these scenarios start on. */
const NODE = 'a-control-plane'

/** The look-alike: the name of a node, none of the identity. */
const LOOKALIKE = 'kind-control-plane'

let seq = 400
const ev = (data: Parameters<typeof stream>[2]) => batch([stream(seq++, globalRoute(), data)])

const kindCluster = (name: string): KindClusterSummary => ({
  name,
  status: 'ready',
  profile: null,
  context_name: `kind-${name}`,
})

/** Select a container row, which is what opens its verbs. */
async function select(page: Page, name: string): Promise<void> {
  await page.getByTestId(`container-row-${name}`).click()
  await expect(page.getByTestId(`container-actions-${name}`)).toBeVisible()
}

test.describe('US4 containers and clusters are one world', () => {
  test('a node container names its cluster and the badge leads to it', async ({ page }) => {
    await scene(page, 'containers-cluster-connected')

    const badge = page.getByTestId(`container-cluster-${NODE}`)
    await expect(badge).toHaveAttribute('data-cluster', 'a')
    await expect(badge).toHaveText('node of a')

    // The link is the operator's own route to the cluster, so it is clicked rather than asserted
    // as an attribute: a badge that looks like a link and goes nowhere answers nothing.
    await badge.click()
    await expect(page.getByTestId('kind-clusters')).toBeVisible()
  })

  test('a look-alike carrying no cluster label has no cluster claimed', async ({ page }) => {
    await scene(page, 'containers-cluster-connected')

    // It is on screen — this is not a test that passes because the container is missing.
    await expect(page.getByTestId(`container-row-${LOOKALIKE}`)).toBeVisible()
    await expect(page.getByTestId(`container-cluster-${LOOKALIKE}`)).toHaveCount(0)
    // And exactly one container in the whole corpus claims a cluster.
    const claimed = CLUSTER_CONTAINERS.filter((c) => c.cluster !== null)
    await expect(page.locator('[data-testid^="container-cluster-"]')).toHaveCount(claimed.length)
  })

  test('the cluster lists the containers backing it and links back', async ({ page }) => {
    await open(page, 'containers-cluster-connected')
    await navigateTo(page, 'Local clusters', 'Settings')
    await expect(page.getByTestId('kind-clusters')).toBeVisible()
    await emit(page, ev({ type: 'KindClustersUpdated', data: { clusters: [kindCluster('a')] } }))

    // The cluster view asks for the containers itself: nothing polls the containers scene while
    // nobody is looking at it, so a cluster opened first would otherwise have no link to read.
    await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerInventory')

    const nodes = page.getByTestId('kind-nodes-a')
    await expect(nodes).toBeVisible()
    const node = nodes.getByTestId(`kind-node-${NODE}`)
    await expect(node).toHaveAttribute('data-state', 'running')
    await expect(node).toContainText('Up 3 hours')
    // Nothing claims a cluster is degraded while its nodes are up.
    await expect(page.getByTestId('kind-nodes-degraded-a')).toHaveCount(0)

    await page.getByTestId('kind-open-containers-a').click()
    await expect(page.getByTestId('container-group-list')).toBeVisible()
    await expect(page.getByTestId(`container-row-${NODE}`)).toBeVisible()
  })

  test('stopping a backing node warns, names the cluster, and says you are in it', async ({
    page,
  }) => {
    await scene(page, 'containers-cluster-connected')
    await select(page, NODE)
    await page.getByTestId(`container-stop-${NODE}`).click()

    const warning = page.getByTestId('operation-breaks-clusters')
    await expect(warning).toBeVisible()
    await expect(warning).toContainText('a loses a node this acts on')
    await expect(warning).toContainText('you are connected to it right now')
  })

  test('the same stop from another context warns without claiming you are connected', async ({
    page,
  }) => {
    // `containers-adopted` has its own node, of `fathom-dev`, while the operator is on `kind-a`.
    // The break is the same break; the second sentence is not true and must not be said.
    await scene(page, 'containers-adopted')
    await select(page, 'fathom-dev-control-plane')
    await page.getByTestId('container-stop-fathom-dev-control-plane').click()

    const warning = page.getByTestId('operation-breaks-clusters')
    await expect(warning).toContainText('fathom-dev loses a node this acts on')
    await expect(warning).not.toContainText('connected')
  })

  test('starting a node back up warns about nothing', async ({ page }) => {
    // A warning on the verb that *repairs* the cluster is how an operator learns the warning
    // means nothing, so `start` carries none.
    await scene(page, 'containers-node-stopped')
    await select(page, NODE)
    await page.getByTestId(`container-start-${NODE}`).click()
    await expect(page.getByTestId('operation-preview')).toBeVisible()
    await expect(page.getByTestId('operation-breaks-clusters')).toHaveCount(0)
  })

  test('a node stopped outside Fathom is reflected on both sides', async ({ page }) => {
    await scene(page, 'containers-node-stopped')
    const row = page.getByTestId(`container-row-${NODE}`)
    await expect(row).toHaveAttribute('data-state', 'exited')
    await expect(row).toContainText('Exited (0) 2 minutes ago')

    await navigateTo(page, 'Local clusters', 'Settings')
    await emit(page, ev({ type: 'KindClustersUpdated', data: { clusters: [kindCluster('a')] } }))

    // kind lists a cluster whose nodes are all stopped exactly as it lists a working one, so the
    // row's own "ready" is not the whole truth and the view must not leave it as the whole truth.
    const degraded = page.getByTestId('kind-nodes-degraded-a')
    await expect(degraded).toContainText('1 of 1 node is not running')
    await expect(degraded).toContainText('cannot be relied on')
    await expect(page.getByTestId(`kind-node-${NODE}`)).toHaveAttribute('data-state', 'exited')
  })
})

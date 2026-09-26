/**
 * 030-kind-cluster-provisioning: prerequisites, inventory, create (confirm + progress),
 * name validation, and delete (destructive confirm), against the scripted fake backend.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, stream, system } from '../fixtures/builders'
import type { KindClusterSummary, KindPrerequisites } from '../../src/protocol/types'
import { clearSent, emit, navigateTo, open, sent, sentTypes } from './helpers'

const panel = (page: Page) => page.getByTestId('kind-clusters')

const READY: KindPrerequisites = {
  kind_installed: true,
  kind_version: 'kind v0.23.0',
  runtime_installed: true,
  runtime_kind: 'docker',
  runtime_running: true,
  ready: true,
}

const cluster = (name: string): KindClusterSummary => ({
  name,
  status: 'ready',
  profile: null,
  context_name: `kind-${name}`,
})

const BASE_SETTINGS = {
  schema: 1 as const,
  metricsSource: 'none' as const,
  prometheusPath: '/api/v1',
  https: false,
  pathPrefix: '',
  hiddenMetrics: [],
  orgRevKnown: null,
  fieldOrigin: {
    metricsSource: 'local' as const,
    prometheusPath: 'local' as const,
    https: 'local' as const,
    pathPrefix: 'local' as const,
    hiddenMetrics: 'local' as const,
  },
}

let seq = 100
const ev = (data: Parameters<typeof stream>[2]) => batch([stream(seq++, globalRoute(), data)])

async function goToPanel(page: Page) {
  await open(page)
  await navigateTo(page, 'Cluster Settings', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
  await emit(
    page,
    batch([
      system(1, {
        type: 'ClusterSettingsLoaded',
        data: { context: 'kind-a', settings: BASE_SETTINGS },
      }),
    ]),
  )
  await expect(panel(page)).toBeVisible()
  await expect.poll(() => sentTypes(page)).toContain('RefreshKindPrerequisites')
  await expect.poll(() => sentTypes(page)).toContain('RefreshKindClusters')
}

test.describe('030 kind cluster provisioning', () => {
  test('missing kind shows install guidance and disables create', async ({ page }) => {
    await goToPanel(page)
    await emit(
      page,
      ev({
        type: 'KindPrerequisitesUpdated',
        data: {
          prerequisites: {
            ...READY,
            kind_installed: false,
            kind_version: null,
            ready: false,
          },
        },
      }),
    )
    await expect(panel(page).getByTestId('kind-missing')).toContainText('kind is not installed')
    await panel(page).getByTestId('kind-name-input').fill('dev')
    await expect(panel(page).getByTestId('kind-create')).toBeDisabled()
  })

  test('runtime installed but stopped is distinct from missing', async ({ page }) => {
    await goToPanel(page)
    await emit(
      page,
      ev({
        type: 'KindPrerequisitesUpdated',
        data: { prerequisites: { ...READY, runtime_running: false, ready: false } },
      }),
    )
    await expect(panel(page).getByTestId('kind-runtime-stopped')).toContainText('not running')
  })

  test('empty state then list rendering', async ({ page }) => {
    await goToPanel(page)
    await emit(page, ev({ type: 'KindClustersUpdated', data: { clusters: [] } }))
    await expect(panel(page).getByTestId('kind-empty')).toBeVisible()
    await emit(
      page,
      ev({ type: 'KindClustersUpdated', data: { clusters: [cluster('a'), cluster('b')] } }),
    )
    await expect(panel(page).getByTestId('kind-row')).toHaveCount(2)
  })

  test('invalid and colliding names are rejected inline before confirmation', async ({ page }) => {
    await goToPanel(page)
    await emit(page, ev({ type: 'KindPrerequisitesUpdated', data: { prerequisites: READY } }))
    await emit(page, ev({ type: 'KindClustersUpdated', data: { clusters: [cluster('dev')] } }))
    const input = panel(page).getByTestId('kind-name-input')
    await input.fill('Bad_Name')
    await expect(panel(page).getByTestId('kind-name-error')).toBeVisible()
    await expect(panel(page).getByTestId('kind-create')).toBeDisabled()
    await input.fill('dev')
    await expect(panel(page).getByTestId('kind-name-error')).toContainText('already exists')
    await expect(panel(page).getByTestId('kind-create')).toBeDisabled()
    expect(await sentTypes(page)).not.toContain('CreateKindCluster')
  })

  test('create: confirm summary, then progress, then cluster appears', async ({ page }) => {
    await goToPanel(page)
    await emit(page, ev({ type: 'KindPrerequisitesUpdated', data: { prerequisites: READY } }))
    await emit(page, ev({ type: 'KindClustersUpdated', data: { clusters: [] } }))
    await panel(page).getByTestId('kind-name-input').fill('demo')
    await clearSent(page)
    await panel(page).getByTestId('kind-create').click()

    const dialog = page.getByTestId('confirm-dialog')
    await expect(dialog).toContainText('demo')
    await expect(page.getByTestId('kind-create-summary')).toContainText('bare single-node')
    expect(await sentTypes(page)).not.toContain('CreateKindCluster')

    await page.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('CreateKindCluster')
    const cmd = (await sent(page)).find((c) => c.type === 'CreateKindCluster')
    if (cmd?.type !== 'CreateKindCluster') throw new Error('missing command')
    expect(cmd.data.name).toBe('demo')
    expect(cmd.data.profile).toBe('bare')
    const requestId = cmd.data.request_id

    await emit(
      page,
      ev({
        type: 'KindProvisionProgress',
        data: { request_id: requestId, stage: 'Preparing nodes' },
      }),
    )
    await expect(panel(page).getByTestId('kind-progress')).toContainText('Preparing nodes')
    await emit(
      page,
      ev({
        type: 'KindProvisionProgress',
        data: { request_id: requestId, stage: 'Starting control-plane' },
      }),
    )
    await expect(panel(page).getByTestId('kind-progress')).toContainText('Starting control-plane')
    // kind is silent for ~10 s here on a real host: the elapsed counter keeps moving (SC-004).
    const elapsed = panel(page).getByTestId('kind-progress-elapsed')
    const first = await elapsed.textContent()
    await expect(elapsed).not.toHaveText(first ?? '', { timeout: 3000 })

    await emit(
      page,
      batch([
        system(seq++, {
          type: 'ContextsUpdated',
          data: { contexts: ['kind-a', 'kind-b', 'kind-demo'], current: 'kind-a' },
        }),
        stream(seq++, globalRoute(), {
          type: 'KindClustersUpdated',
          data: { clusters: [cluster('demo')] },
        }),
        stream(seq++, globalRoute(), {
          type: 'KindClusterActionFinished',
          data: { request_id: requestId, ok: true, error: null },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('kind-row')).toHaveCount(1)
    await expect(panel(page).getByTestId('kind-progress')).toHaveAttribute(
      'data-status',
      'complete',
    )
    if ((await page.locator('html').getAttribute('data-width-class')) === 'phone')
      await page.getByTestId('scope-chip').click()
    await page.getByRole('combobox', { name: 'Context' }).click()
    await expect(page.getByRole('option', { name: 'kind-demo' })).toBeVisible()
  })

  test('delete: destructive confirm names the cluster and blocks until confirmed', async ({
    page,
  }) => {
    await goToPanel(page)
    await emit(page, ev({ type: 'KindPrerequisitesUpdated', data: { prerequisites: READY } }))
    await emit(page, ev({ type: 'KindClustersUpdated', data: { clusters: [cluster('old')] } }))
    await clearSent(page)

    await panel(page).getByTestId('kind-delete').click()
    await expect(page.getByTestId('confirm-dialog')).toContainText('old')
    // Cancel: nothing sent.
    await page.getByTestId('confirm-cancel').click()
    expect(await sentTypes(page)).not.toContain('DeleteKindCluster')

    await panel(page).getByTestId('kind-delete').click()
    await page.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('DeleteKindCluster')
    const cmd = (await sent(page)).find((c) => c.type === 'DeleteKindCluster')
    if (cmd?.type !== 'DeleteKindCluster') throw new Error('missing command')
    expect(cmd.data.name).toBe('old')
    // Still listed until the backend confirms.
    await expect(panel(page).getByTestId('kind-row')).toHaveCount(1)

    await emit(
      page,
      batch([
        stream(seq++, globalRoute(), { type: 'KindClustersUpdated', data: { clusters: [] } }),
        stream(seq++, globalRoute(), {
          type: 'KindClusterActionFinished',
          data: { request_id: cmd.data.request_id, ok: true, error: null },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('kind-empty')).toBeVisible()
  })
})

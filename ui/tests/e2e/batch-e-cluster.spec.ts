/** Batch E: Namespaces list, Events panel and cluster Overview panel. */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, clusterRoute, list, read } from '../fixtures/builders'
import type { KubernetesEvent, NamespaceSummary } from '../../src/protocol/types'
import {
  emit,
  expectNoHorizontalScroll,
  expectRowCount,
  inspector,
  navigateTo,
  settleFakeReplies,
  open,
  openToolPane,
  row,
  sentTypes,
} from './helpers'

const NAMESPACES: NamespaceSummary[] = [
  {
    name: 'default',
    status: 'Active',
    pod_count: 4,
    deployment_count: 2,
    creation_timestamp: '2024-01-01T00:00:00Z',
  },
  {
    name: 'kube-system',
    status: 'Active',
    pod_count: 9,
    deployment_count: 3,
    creation_timestamp: '2024-01-01T00:00:00Z',
  },
  {
    name: 'old',
    status: 'Terminating',
    pod_count: 0,
    deployment_count: 0,
    creation_timestamp: '2024-02-01T00:00:00Z',
  },
]

const ago = (s: number): string => new Date(Date.now() - s * 1000).toISOString()

function ev(p: Partial<KubernetesEvent> & { name: string }): KubernetesEvent {
  return {
    namespace: 'default',
    event_type: 'Normal',
    involved_object_kind: 'Pod',
    involved_object_name: 'web-1',
    reason: 'Scheduled',
    message: 'Successfully assigned',
    count: 1,
    first_timestamp: ago(600),
    last_timestamp: ago(600),
    source: 'kubelet',
    ...p,
  }
}

const EVENTS: KubernetesEvent[] = [
  ev({ name: 'e1', reason: 'Scheduled', message: 'Successfully assigned default/web-1' }),
  ev({
    name: 'e2',
    event_type: 'Warning',
    reason: 'BackOff',
    message: 'Back-off restarting failed container',
    involved_object_name: 'web-2',
    count: 5,
    last_timestamp: ago(60),
  }),
  ev({
    name: 'e3',
    namespace: 'kube-system',
    event_type: 'Warning',
    reason: 'FailedMount',
    message: 'Unable to attach volume',
    involved_object_name: 'dns-1',
    last_timestamp: ago(120),
  }),
  ev({
    name: 'e4',
    namespace: 'kube-system',
    reason: 'Pulled',
    involved_object_kind: 'Widget',
    involved_object_name: 'w1',
    message: 'Image pulled',
    last_timestamp: ago(300),
  }),
]

const eventsBatch = (events: KubernetesEvent[] = EVENTS) =>
  batch([
    list(1, clusterRoute('kind-a', 'events'), {
      type: 'EventsUpdated',
      data: { events, total_count: events.length },
    }),
  ])

/**
 * Events is a tool pane (stack tab / dock tab) where one exists; the Overview tool is only docked
 * at ultra, so it, and Events where no tab is offered, are reached through their nav sections
 * (which render the same panels).
 */
async function openTool(page: Page, info: TestInfo, label: 'Events' | 'Overview'): Promise<void> {
  const name = info.project.name
  if (label === 'Events' && !name.endsWith('-laptop')) {
    if (name.endsWith('-phone') || name.endsWith('-tablet')) {
      await openToolPane(page, label)
      return
    }
    const tab = page.getByRole('tab', { name: label })
    if (await tab.count()) {
      await tab.click()
      return
    }
  }
  await openNavSection(page, label === 'Events' ? 'events' : 'overview')
}

/** Nav row by `data-section` (the Overview group and section share a label). */
async function openNavSection(page: Page, section: string): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const drawer = !(await nav.isVisible())
  if (drawer) await page.getByRole('button', { name: 'Open sections' }).click()
  await expect(nav).toBeVisible()
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Overview' })
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator(`button[data-section="${section}"]`).click()
  if (drawer) await expect(nav).toBeHidden()
}

const isPhoneProject = (i: TestInfo): boolean => i.project.name.endsWith('-phone')

test.describe('Namespaces list', () => {
  test('lists namespace rows with status', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Namespaces', 'Overview')
    await expect.poll(() => sentTypes(page)).toContain('RefreshNamespaces')
    await emit(
      page,
      batch([
        list(1, clusterRoute('kind-a', 'system'), {
          type: 'NamespaceSummariesUpdated',
          data: NAMESPACES,
        }),
      ]),
    )
    await expectRowCount(page, 3, 'namespaces')
    await expect(row(page, 'default', 'namespaces')).toBeVisible()
    await expect(row(page, 'old', 'namespaces')).toContainText('Terminating')
    await expect(row(page, 'kube-system', 'namespaces')).toContainText('kube-system')
  })

  test('opening a namespace reads it and shows labels and annotations', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Namespaces', 'Overview')
    await expect.poll(() => sentTypes(page)).toContain('RefreshNamespaces')
    const route = clusterRoute('kind-a', 'namespaces')
    await emit(
      page,
      batch([
        list(1, clusterRoute('kind-a', 'system'), {
          type: 'NamespaceSummariesUpdated',
          data: NAMESPACES,
        }),
      ]),
    )
    await row(page, 'default', 'namespaces').click()
    await expect.poll(() => sentTypes(page)).toContain('SelectNamespace')
    await emit(
      page,
      batch([
        read(2, route, {
          type: 'NamespaceDetailsUpdated',
          data: {
            name: 'default',
            status: 'Active',
            labels: { 'team.example/owner': 'platform' },
            annotations: { 'note.example/purpose': 'shared-sandbox' },
            creation_timestamp: '2024-01-01T00:00:00Z',
            resource_quota_names: ['compute-quota'],
            limit_range_names: [],
            raw_json: '{"kind":"Namespace"}',
          },
        }),
      ]),
    )
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(insp.getByRole('heading', { name: 'Labels' })).toBeVisible()
    await expect(insp.getByRole('heading', { name: 'Annotations' })).toBeVisible()
    await expect(insp).toContainText('team.example/owner')
    await expect(insp).toContainText('platform')
    await expect(insp).toContainText('note.example/purpose')
    await expect(insp).toContainText('shared-sandbox')
    await expect(insp).not.toContainText('Not available')
    await insp.getByRole('tab', { name: 'YAML' }).click()
    await expect(insp).toContainText('Namespace')
  })
})

test.describe('Events panel', () => {
  test('shows warning vs normal, newest first, and filters', async ({ page }, info) => {
    await open(page)
    await openTool(page, info, 'Events')
    await settleFakeReplies(page)
    await emit(page, eventsBatch())
    const panel = page.getByTestId('events-panel').first()
    const rows = panel.locator('[data-event-row]')
    await expect(rows).toHaveCount(4)
    await expect(rows.first()).toContainText('BackOff')
    await expect(rows.filter({ hasText: 'BackOff' })).toContainText('Warning')
    await expect(rows.filter({ hasText: 'Scheduled' })).toContainText('Normal')

    await panel.getByRole('button', { name: 'Warning', exact: true }).click()
    await expect(rows).toHaveCount(2)

    await panel.getByRole('button', { name: 'All', exact: true }).click()
    await panel.getByLabel('Namespace').selectOption('kube-system')
    await expect(rows).toHaveCount(2)
    await panel.getByLabel('Namespace').selectOption('')

    await panel.getByLabel('Search events').fill('volume')
    await expect(rows).toHaveCount(1)
    await expect(rows.first()).toContainText('FailedMount')
    await panel.getByLabel('Search events').fill('nomatchxyz')
    await expect(panel.getByText('No matching events')).toBeVisible()
    await expectNoHorizontalScroll(page)
  })

  test('object link opens the inspector; unknown kinds are not links', async ({ page }, info) => {
    await open(page)
    await openTool(page, info, 'Events')
    await settleFakeReplies(page)
    await emit(page, eventsBatch())
    const panel = page.getByTestId('events-panel').first()
    await expect(panel.getByRole('button', { name: 'Open Widget/w1' })).toHaveCount(0)
    const link = panel.getByRole('button', { name: 'Open Pod/web-2' })
    const box = await link.boundingBox()
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(24)
    await link.click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page)).toContainText('web-2')
  })

  test('phone uses the card layout', async ({ page }, info) => {
    test.skip(!isPhoneProject(info), 'phone only')
    await open(page)
    await openTool(page, info, 'Events')
    await settleFakeReplies(page)
    await emit(page, eventsBatch())
    const cards = page.getByTestId('events-panel').first().locator('li[data-event-row]')
    await expect(cards).toHaveCount(4)
    await expect(cards.first()).toContainText('Back-off restarting failed container')
    await expectNoHorizontalScroll(page)
  })
})

test.describe('Overview panel', () => {
  test('shows summary cards and recent warnings with links', async ({ page }, info) => {
    await open(page)
    await openTool(page, info, 'Overview')
    await settleFakeReplies(page)
    await emit(page, eventsBatch())
    const panel = page.getByTestId('overview-panel').first()
    for (const s of ['nodes', 'pods', 'deployments', 'namespaces', 'warnings']) {
      await expect(panel.locator(`[data-stat="${s}"]`)).toBeVisible()
    }
    await expect(panel.locator('[data-stat="warnings"]')).toContainText('2')
    await expect(panel.locator('[data-stat="server-version"]')).toBeVisible()
    const warnings = panel.getByRole('region', { name: 'Recent warnings' })
    await expect(warnings.getByText('BackOff')).toBeVisible()
    await expect(warnings.getByText('FailedMount')).toBeVisible()
    await expect(warnings.getByText('Scheduled')).toHaveCount(0)
    await expectNoHorizontalScroll(page)
    await warnings.getByRole('button', { name: 'Open Pod/web-2' }).click()
    await expect(inspector(page)).toBeVisible()
    await expect(inspector(page)).toContainText('web-2')
    await expectNoHorizontalScroll(page)
  })
})

/**
 * T044-T049 (US3 Phase 5): generic, read-only Custom Resource Definition browsing against the
 * fake backend. Navigating to "Custom Resources" sends `RefreshCrds`; selecting a kind sends
 * `SelectCrdKind` + `RefreshCrdInstances`; opening an instance sends `SelectCrdInstance` and
 * renders Details/YAML/Events like the built-in-kind Inspector (FR-015..FR-020).
 */
import { expect, test, type Page } from '@playwright/test'

import type { CrdCatalogEntry } from '../../src/protocol/types'
import {
  batch,
  clusterRoute,
  globalRoute,
  list,
  read,
  SECTIONS,
  system,
} from '../fixtures/builders'
import { emit, open, sent, sentTypes } from './helpers'

function catalog(seq: number, entries: CrdCatalogEntry[]) {
  return batch([system(seq, { type: 'CrdCatalogUpdated', data: { entries } })])
}

function instances(
  seq: number,
  group: string,
  kind: string,
  version: string,
  namespace: string | null,
  items: string[],
) {
  return batch([
    system(seq, {
      type: 'CrdInstancesUpdated',
      data: { group, kind, version, namespace, instances: items, failure: null },
    }),
  ])
}

const WIDGET: CrdCatalogEntry = {
  group: 'example.com',
  kind: 'Widget',
  plural: 'widgets',
  versions: ['v1'],
  scope: 'Namespaced',
  permission_denied: false,
}

const GADGET: CrdCatalogEntry = {
  group: 'example.com',
  kind: 'Gadget',
  plural: 'gadgets',
  versions: ['v1'],
  scope: 'Cluster',
  permission_denied: false,
}

const SECRETTHING: CrdCatalogEntry = {
  group: 'ops.example.com',
  kind: 'SecretThing',
  plural: 'secretthings',
  versions: ['v1'],
  scope: 'Namespaced',
  permission_denied: true,
}

function widgetJson(namespace: string, name: string) {
  return JSON.stringify({
    apiVersion: 'example.com/v1',
    kind: 'Widget',
    metadata: { namespace, name, resourceVersion: '123' },
    spec: { size: 'large' },
  })
}

function gadgetJson(name: string) {
  return JSON.stringify({
    apiVersion: 'example.com/v1',
    kind: 'Gadget',
    metadata: { name, resourceVersion: '9' },
    spec: {},
  })
}

// The "Custom Resources" group has a single section whose label is also "Custom Resources",
// so `helpers.navigateTo` (name-based, ambiguous here) is not reused; the toggle and the item
// are disambiguated by `aria-expanded` and `data-section` instead.
async function goToCustomResources(page: Page) {
  await open(page)
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.getByRole('button', { name: 'Custom Resources', expanded: false })
  if (await toggle.isVisible()) await toggle.click()
  await nav.locator('[data-section="custom-resources"]').click()
  await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['RefreshCrds']))
}

test.describe('US3 Custom Resource browsing', () => {
  test('T044: catalog lists every installed CRD kind, not just one integration', async ({
    page,
  }) => {
    await goToCustomResources(page)
    await emit(page, catalog(100, [WIDGET, GADGET, SECRETTHING]))

    await expect(page.getByRole('option', { name: /Widget/ })).toBeVisible()
    await expect(page.getByRole('option', { name: /Gadget/ })).toBeVisible()
    await expect(page.getByRole('option', { name: /SecretThing/ })).toBeVisible()
  })

  test('T045: selecting a kind fetches correctly scoped instances (namespaced vs cluster)', async ({
    page,
  }) => {
    await goToCustomResources(page)
    await emit(page, catalog(100, [WIDGET, GADGET]))

    await page.getByRole('option', { name: /Widget/ }).click()
    await expect
      .poll(() => sentTypes(page))
      .toEqual(expect.arrayContaining(['SelectCrdKind', 'RefreshCrdInstances']))
    const refresh = (await sent(page)).find((c) => c.type === 'RefreshCrdInstances')
    expect(refresh).toMatchObject({
      type: 'RefreshCrdInstances',
      data: { group: 'example.com', kind: 'Widget' },
    })

    await emit(
      page,
      instances(101, 'example.com', 'Widget', 'v1', null, [widgetJson('team-a', 'widget-one')]),
    )
    await expect(page.locator('[role="row"][data-row-key="team-a/widget-one"]')).toBeVisible()
    await expect(page.locator('[role="row"][data-row-key="team-a/widget-one"]')).toContainText(
      'team-a',
    )

    // Cluster-scoped: instances carry no namespace.
    await page.getByRole('option', { name: /Gadget/ }).click()
    await emit(
      page,
      instances(102, 'example.com', 'Gadget', 'v1', null, [gadgetJson('gadget-one')]),
    )
    const row = page.locator('[role="row"][data-row-key="gadget-one"]')
    await expect(row).toBeVisible()
    await expect(row).toContainText('—')
  })

  test('T046: opening an instance renders Details/YAML/Events via the generic detail view', async ({
    page,
  }) => {
    await goToCustomResources(page)
    await emit(page, catalog(100, [WIDGET]))
    await page.getByRole('option', { name: /Widget/ }).click()
    await emit(
      page,
      instances(101, 'example.com', 'Widget', 'v1', null, [widgetJson('team-a', 'widget-one')]),
    )

    await page.locator('[role="row"][data-row-key="team-a/widget-one"]').click()
    await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['SelectCrdInstance']))

    await emit(
      page,
      batch([
        read(200, globalRoute('custom-resources'), {
          type: 'CrdInstanceDetailsUpdated',
          data: {
            group: 'example.com',
            kind: 'Widget',
            version: 'v1',
            namespace: 'team-a',
            name: 'widget-one',
            resource_version: '123',
            raw_json: widgetJson('team-a', 'widget-one'),
          },
        }),
      ]),
    )

    const detail = page.getByTestId('crd-instance-detail')
    await expect(detail.getByRole('tab', { name: 'Details' })).toBeVisible()
    await expect(detail.getByText('widget-one').first()).toBeVisible()
    await detail.getByRole('tab', { name: 'YAML' }).click()
    await expect(detail.getByText(/kind: Widget/)).toBeVisible()

    // CrdInstanceEvents reads the cluster events list (like the built-in Inspector's EventsFor)
    // rather than fetching its own; the events section must have loaded for a definite state.
    await emit(
      page,
      batch([
        list(201, clusterRoute('kind-a', SECTIONS.events), {
          type: 'EventsUpdated',
          data: { events: [], total_count: 0 },
        }),
      ]),
    )
    await detail.getByRole('tab', { name: 'Events' }).click()
    await expect(detail.getByText('No events')).toBeVisible()
  })

  test('T047: zero installed CRDs shows an explicit empty state, not an error', async ({
    page,
  }) => {
    await goToCustomResources(page)
    await emit(page, catalog(100, []))
    await expect(page.getByText('No Custom Resource Definitions')).toBeVisible()
    await expect(page.getByText(/error/i)).toHaveCount(0)
  })

  test("T048: one CRD's permission-denied is isolated to that CRD; others list normally", async ({
    page,
  }) => {
    await goToCustomResources(page)
    await emit(page, catalog(100, [WIDGET, SECRETTHING]))

    await expect(page.getByRole('option', { name: /SecretThing/ })).toContainText(
      'permission denied',
    )
    await expect(page.getByRole('option', { name: /Widget/ })).not.toContainText(
      'permission denied',
    )

    await page.getByRole('option', { name: /SecretThing/ }).click()
    await expect(page.getByText('Permission denied', { exact: true })).toBeVisible()

    await page.getByRole('option', { name: /Widget/ }).click()
    await emit(
      page,
      instances(101, 'example.com', 'Widget', 'v1', null, [widgetJson('team-a', 'widget-one')]),
    )
    await expect(page.locator('[role="row"][data-row-key="team-a/widget-one"]')).toBeVisible()
  })

  test('T049: a CRD deleted mid-view (dropped from a fresh catalog) shows "no longer available"', async ({
    page,
  }) => {
    await goToCustomResources(page)
    await emit(page, catalog(100, [WIDGET, GADGET]))
    await page.getByRole('option', { name: /Widget/ }).click()
    await emit(
      page,
      instances(101, 'example.com', 'Widget', 'v1', null, [widgetJson('team-a', 'widget-one')]),
    )
    await expect(page.locator('[role="row"][data-row-key="team-a/widget-one"]')).toBeVisible()

    // Widget is no longer in a fresh catalog (its CRD was deleted).
    await emit(page, catalog(102, [GADGET]))

    await expect(page.getByText('Widget is no longer available')).toBeVisible()
    await expect(page.locator('[role="row"][data-row-key="team-a/widget-one"]')).toHaveCount(0)
  })
})

/**
 * US1/US2 (017-cluster-linter): open the Linter pane, see a finding for a known-bad Pod (naked,
 * no limits) computed from list rows + per-object reads, click it, and land in the Inspector on
 * that exact Pod. The pane must only ever send read commands.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, list, nsRoute, read } from '../fixtures/builders'
import { emit, inspector, open, sentTypes } from './helpers'

const PODS = 'workloads/pods'

async function openLinter(page: Page, info: TestInfo): Promise<void> {
  const stacked = info.project.name.endsWith('-phone') || info.project.name.endsWith('-tablet')
  if (stacked) {
    await page.getByRole('navigation', { name: 'Views' }).getByRole('button', { name: 'Linter' }).click()
  } else {
    await page.getByRole('tab', { name: 'Linter' }).click()
  }
}

const POD_DETAILS = {
  namespace: 'default',
  name: 'naked-pod',
  creation_timestamp: '2024-01-15T10:00:00Z',
  labels: {},
  annotations: {},
  containers: ['app'],
  ports: [],
  resource_specs: [],
  owner: null,
  resolved_owner: null,
  persistent_volume_claims: [],
  containers_lifecycle: [],
  raw_json: JSON.stringify({ metadata: { name: 'naked-pod', namespace: 'default' }, spec: { containers: [{ name: 'app', image: 'nginx' }] } }),
}

test.describe('US1 linter findings', () => {
  test('lists a finding for a naked pod without limits and opens the inspector on click', async ({ page }, info) => {
    await open(page)
    await openLinter(page, info)
    await expect(page.getByTestId('linter-panel')).toBeVisible()

    // Mount refreshes lists with existing read-only commands only.
    await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['RefreshPods', 'RefreshDeployments', 'RefreshRoles']))

    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', PODS), {
          type: 'PodsUpdated',
          data: [{ name: 'naked-pod', namespace: 'default', phase: 'Running', node: 'n1', ready: 1, total: 1, restarts: 0 }],
        }),
      ]),
    )
    // The linter reads the pod's details itself (existing SelectPod command).
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')
    await emit(page, batch([read(2, nsRoute('kind-a', PODS), { type: 'PodDetailsUpdated', data: POD_DETAILS })]))

    const finding = page.getByTestId('linter-finding').filter({ hasText: 'naked-pod' }).first()
    await expect(finding).toBeVisible()
    await expect(page.getByTestId('linter-finding').filter({ hasText: 'no owning controller' })).toHaveCount(1)

    // Skipped checks are surfaced rather than silently dropped (ServiceAccounts are unavailable).
    await expect(page.getByTestId('linter-skipped')).toContainText('ServiceAccounts')

    // Severity filter narrows the list; clearing restores it.
    await page.getByTestId('linter-severity-low').click()
    await expect(page.getByTestId('linter-finding')).toHaveCount(0)
    await page.getByTestId('linter-clear-filters').click()
    await expect(finding).toBeVisible()

    await finding.click()
    await expect(inspector(page)).toContainText('naked-pod')

    const types = await sentTypes(page)
    expect(types.filter((t) => !/^(Refresh|Select|Set|Load|Initialize)/.test(t))).toEqual([])
  })
})

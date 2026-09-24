/**
 * 018-rbac-aware-mutation-ui US1: node action controls reflect the operator's real permissions
 * before any confirm dialog opens. A denied `update nodes` review disables the controls with the
 * API server's reason; an allowed one leaves them enabled; no answer yet leaves them enabled.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, list, read } from '../fixtures/builders'
import {
  emit,
  inspector,
  open,
  sentTypes,
} from './helpers'

const NODE_A = {
  name: 'kind-a-worker',
  status: 'Ready',
  role: 'worker',
  pod_count: 8,
  pod_capacity: 110,
  cpu_allocatable_millicores: 2000,
  memory_allocatable_bytes: 4_000_000_000,
}

const NODES_ROUTE = clusterRoute('kind-a', 'nodes')

async function goToNode(page: Page): Promise<void> {
  await open(page)
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open navigator' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.getByRole('button', { name: 'Overview', expanded: false })
  if (await toggle.isVisible()) await toggle.click()
  await nav.locator('[data-section="nodes"]').click()
  await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['RefreshNodes']))
  await emit(page, batch([list(1, NODES_ROUTE, { type: 'NodesUpdated', data: [NODE_A] })]))
  await page.locator(`[role="row"][data-row-key="${NODE_A.name}"]`).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await insp.getByRole('tab', { name: 'Actions' }).click()
  // Expanding the nav group persists a debounced `SaveUiConfig`; let it settle before the
  // test starts asserting on `sent()`, or it pollutes the very next assertion.
  await expect.poll(() => sentTypes(page)).toContain('SaveUiConfig')
}


async function answer(page: Page, allowed: boolean, reason: string | null): Promise<void> {
  await emit(
    page,
    batch([
      read(100, NODES_ROUTE, {
        type: 'RbacSelfAccessUpdated',
        data: {
          results: [
            {
              check: { verb: 'update', resource: 'nodes', namespace: null, group: '' },
              allowed,
              reason,
            },
          ],
        },
      }),
    ]),
  )
}

test.describe('018 mutation access gating', () => {
  test('denied identity: node actions are disabled with the reason, no dialog', async ({ page }) => {
    await goToNode(page)
    await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
    await answer(page, false, 'RBAC: nodes is forbidden')
    const cordon = page.getByTestId('node-action-cordon')
    await expect(cordon).toBeDisabled()
    await expect(page.getByTestId('node-action-drain')).toBeDisabled()
    await expect(page.getByTestId('access-denied-hint').first()).toBeVisible()
    await page.getByTestId('access-denied-hint').first().focus()
    await expect(page.getByRole('tooltip').first()).toContainText('RBAC: nodes is forbidden')
  })

  test('allowed identity: node actions stay enabled', async ({ page }) => {
    await goToNode(page)
    await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
    await answer(page, true, null)
    await expect(page.getByTestId('node-action-cordon')).toBeEnabled()
    await expect(page.getByTestId('access-denied-hint')).toHaveCount(0)
  })

  test('no answer yet: node actions stay enabled with no false tooltip', async ({ page }) => {
    await goToNode(page)
    await expect(page.getByTestId('node-action-cordon')).toBeEnabled()
    await expect(page.getByTestId('access-denied-hint')).toHaveCount(0)
  })
})

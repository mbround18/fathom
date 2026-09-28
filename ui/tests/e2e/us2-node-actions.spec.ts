/**
 * T027-T032 (US2 Phase 4): node scheduling actions (cordon/uncordon), drain with live progress,
 * PDB-blocked pods, bare-pod refusal, cancel mid-drain, and taint add/remove — all behind the
 * same single-click `ConfirmDialog` every other mutating action uses (never typed-count, even
 * for drain: the dialog only ever names the one node).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, list, stream } from '../fixtures/builders'
import {
  actionToast,
  clearSent,
  confirmDialog,
  emit,
  finish,
  inspector,
  open,
  requestIdOf,
  sentIgnoringNodePoll,
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
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.getByRole('button', { name: 'Cluster', expanded: false })
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
  await expect.poll(() => sentTypes(page, true)).toContain('SaveUiConfig')
  // On phone/tablet the row click hides the Nodes list, and its node-metrics poll is released
  // 1.5s later (`nodePolling.ts`); that `StopNodeMetricsPolling` can land at any point in a test.
  // So tests read the id the fake actually assigned (`requestIdOf`) instead of predicting one,
  // and compare sent commands with `sentIgnoringNodePoll`.
}

test.describe('US2 node actions', () => {
  test('T027: cordon and uncordon round trip through the confirm dialog', async ({ page }) => {
    await goToNode(page)
    await clearSent(page)

    await page.getByTestId('node-action-cordon').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-typed-count')).toHaveCount(0)
    await dialog.getByTestId('confirm-submit').click()
    await expect(dialog).toBeHidden()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([{ type: 'CordonNode', data: { name: NODE_A.name } }])
    await finish(page, await requestIdOf(page, 'CordonNode'), true)
    await expect(actionToast(page, 'success').first()).toContainText('Cordon')

    await clearSent(page)
    await page.getByTestId('node-action-uncordon').click()
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([{ type: 'UncordonNode', data: { name: NODE_A.name } }])
    await finish(page, await requestIdOf(page, 'UncordonNode'), true)
    await expect(actionToast(page, 'success').filter({ hasText: 'Uncordon' })).toBeVisible()
  })

  test('T028/T029: drain shows incremental progress and a PDB-blocked pod verbatim', async ({
    page,
  }) => {
    await goToNode(page)
    await clearSent(page)

    await page.getByTestId('node-action-drain').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-typed-count')).toHaveCount(0)
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        {
          type: 'DrainNode',
          data: { name: NODE_A.name, ignore_daemonsets: true, force_delete_empty_dir: false },
        },
      ])
    const drainId = await requestIdOf(page, 'DrainNode')

    await emit(
      page,
      batch([
        stream(2, NODES_ROUTE, {
          type: 'DrainProgress',
          data: {
            request_id: String(drainId),
            total_pods: 3,
            evicted: ['default/api-1'],
            blocked_on: [['kube-system/coredns-1', '429 Too Many Requests: Cannot evict pod']],
            ignored_daemonset_pods: ['kube-system/kube-proxy-abcde'],
          },
        }),
      ]),
    )

    const progress = page.getByTestId('drain-progress')
    await expect(progress).toContainText('1/3')
    await expect(page.getByTestId('drain-blocked')).toContainText(
      '429 Too Many Requests: Cannot evict pod',
    )
    await expect(page.getByTestId('drain-ignored-daemonsets')).toContainText(
      'kube-system/kube-proxy-abcde',
    )

    await emit(
      page,
      batch([
        stream(3, NODES_ROUTE, {
          type: 'NodeActionFinished',
          data: { request_id: String(drainId), ok: true, error: null },
        }),
      ]),
    )
    await expect(progress).toContainText('done')
  })

  test('T030: a bare pod refuses the drain before any eviction, verbatim reason', async ({
    page,
  }) => {
    await goToNode(page)
    await clearSent(page)

    await page.getByTestId('node-action-drain').click()
    const dialog = confirmDialog(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('DrainNode')
    const drainId = await requestIdOf(page, 'DrainNode')

    await emit(
      page,
      batch([
        stream(2, NODES_ROUTE, {
          type: 'NodeActionFinished',
          data: {
            request_id: String(drainId),
            ok: false,
            error: 'refusing to drain: pod default/orphan-1 has no owning controller',
          },
        }),
      ]),
    )
    await expect(page.getByTestId('node-action-error')).toContainText('no owning controller')
  })

  test('T031: cancelling mid-drain preserves partial progress', async ({ page }) => {
    await goToNode(page)
    await clearSent(page)

    await page.getByTestId('node-action-drain').click()
    const dialog = confirmDialog(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('DrainNode')
    const drainId = await requestIdOf(page, 'DrainNode')

    await emit(
      page,
      batch([
        stream(2, NODES_ROUTE, {
          type: 'DrainProgress',
          data: {
            request_id: String(drainId),
            total_pods: 4,
            evicted: ['default/api-1', 'default/api-2'],
            blocked_on: [],
            ignored_daemonset_pods: [],
          },
        }),
      ]),
    )
    await expect(page.getByTestId('drain-progress')).toContainText('2/4')

    await clearSent(page)
    await page.getByTestId('node-action-cancel-drain').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        { type: 'CancelDrainNode', data: { name: NODE_A.name, request_id: String(drainId) } },
      ])
    // Optimistic cancel — the progress made so far stays visible, not reset to zero.
    await expect(page.getByTestId('drain-progress')).toContainText('2/4')
    await expect(page.getByTestId('drain-progress')).toContainText('cancelled')
  })

  test('T032: add and remove a taint, both confirmed', async ({ page }) => {
    await goToNode(page)
    await clearSent(page)

    await page.getByTestId('taint-key').fill('dedicated')
    await page.getByTestId('taint-value').fill('gpu')
    await page.getByTestId('taint-effect').selectOption('NoSchedule')
    await page.getByTestId('node-action-taint-add').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-typed-count')).toHaveCount(0)
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        {
          type: 'TaintNode',
          data: {
            name: NODE_A.name,
            op: 'Add',
            key: 'dedicated',
            value: 'gpu',
            effect: 'NoSchedule',
          },
        },
      ])
    await finish(page, await requestIdOf(page, 'TaintNode'), true)
    await expect(actionToast(page, 'success').first()).toContainText('Add taint')

    await clearSent(page)
    await page.getByTestId('node-action-taint-remove').click()
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        {
          type: 'TaintNode',
          data: {
            name: NODE_A.name,
            op: 'Remove',
            key: 'dedicated',
            value: 'gpu',
            effect: 'NoSchedule',
          },
        },
      ])
    await finish(page, await requestIdOf(page, 'TaintNode'), true)
    await expect(actionToast(page, 'success').filter({ hasText: 'Remove taint' })).toBeVisible()
  })
})

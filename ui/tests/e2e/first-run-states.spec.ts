/**
 * T126 (Linux first run): with no kubeconfig, an empty or malformed one, no current context, an
 * unreachable cluster or rejected credentials, every pane shows one explained state with a next
 * step (create a local kind cluster, pick a context, reload, retry): never a blank pane, a
 * skeleton that never resolves, or a raw error wall.
 */
import { expect, test, type Page } from '@playwright/test'

import {
  MALFORMED_DETAIL,
  NO_KUBECONFIG_PATH,
  PODS_KIND_B,
  UNREACHABLE_DETAIL,
  UNREACHABLE_SERVER,
} from '../fixtures'
import { batch, globalRoute, stream, system } from '../fixtures/builders'
import { clearSent, emit, expectRowCount, listPane, open, sent, sentTypes } from './helpers'

const firstRun = (page: Page) => listPane(page).getByTestId('first-run')

/** No pane is left on a skeleton, and nothing shows a raw list error wall. */
async function expectSettled(page: Page) {
  await expect(page.locator('[data-slot="pane"] [data-pane-state="loading"]')).toHaveCount(0)
  await expect(page.locator('[data-slot="pane"] [data-pane-state="error"]')).toHaveCount(0)
}

test.describe('T126 first-run states', () => {
  test('no kubeconfig: names the searched path and offers a local cluster', async ({ page }) => {
    await open(page, 'no-kubeconfig')
    await expect(firstRun(page)).toHaveAttribute('data-first-run', 'kubeconfig')
    await expect(firstRun(page)).toContainText('No kubeconfig found')
    await expect(firstRun(page)).toContainText(NO_KUBECONFIG_PATH)
    await expectSettled(page)
    // Not an error: the status bar stays quiet.
    await expect(page.getByTestId('status-notice')).toHaveCount(0)

    await clearSent(page)
    await firstRun(page).getByTestId('first-run-reload').click()
    await expect.poll(() => sentTypes(page)).toContain('Initialize')
    // Feedback #1: an unchanged result still gets an answer, naming where Fathom looked.
    await expect(firstRun(page).getByTestId('first-run-reload-outcome')).toHaveText(
      `No kubeconfig found; looked in ${NO_KUBECONFIG_PATH}.`,
    )
    await expect(firstRun(page).getByTestId('first-run-reload')).toBeEnabled()
    await expect(firstRun(page).getByTestId('first-run-paths')).toContainText(NO_KUBECONFIG_PATH)
  })

  test('no kubeconfig: "Create a local cluster" opens kind provisioning, and a created cluster is connected', async ({
    page,
  }) => {
    await open(page, 'no-kubeconfig')
    await firstRun(page).getByTestId('first-run-create-kind').click()
    const settings = page.getByTestId('cluster-settings-first-run')
    await expect(settings).toBeVisible()
    await expect(settings.getByTestId('kind-clusters')).toBeVisible()
    await expect(settings.getByTestId('first-run-create-kind')).toHaveCount(0)
    await expect.poll(() => sentTypes(page)).toContain('RefreshKindPrerequisites')

    // Create through the panel (prerequisites ready), then the backend finishes it.
    await emit(
      page,
      batch([
        stream(900, globalRoute(), {
          type: 'KindPrerequisitesUpdated',
          data: {
            prerequisites: {
              kind_installed: true,
              kind_version: 'kind v0.23.0',
              runtime_installed: true,
              runtime_kind: 'docker',
              runtime_running: true,
              ready: true,
            },
          },
        }),
      ]),
    )
    await emit(
      page,
      batch([stream(899, globalRoute(), { type: 'KindClustersUpdated', data: { clusters: [] } })]),
    )
    await settings.getByTestId('kind-name-input').fill('dev')
    await settings.getByTestId('kind-create').click()
    await page.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('CreateKindCluster')
    const create = (await sent(page)).find((c) => c.type === 'CreateKindCluster')
    if (create?.type !== 'CreateKindCluster') throw new Error('missing CreateKindCluster')
    await clearSent(page)
    await emit(
      page,
      batch([
        system(901, {
          type: 'ContextsUpdated',
          data: { contexts: ['kind-dev'], current: 'kind-dev' },
        }),
        system(902, { type: 'KubeconfigStatus', data: { issue: null, paths: [] } }),
        stream(903, globalRoute(), {
          type: 'KindClusterActionFinished',
          data: { request_id: create.data.request_id, ok: true, error: null },
        }),
      ]),
    )
    await expect
      .poll(async () => (await sent(page)).filter((c) => c.type === 'SetContext'))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-dev' } }])
  })

  test('malformed kubeconfig: shows the parser detail and how to fix it', async ({ page }) => {
    await open(page, 'kubeconfig-malformed')
    await expect(firstRun(page)).toContainText('Kubeconfig could not be parsed')
    await expect(firstRun(page)).toContainText(MALFORMED_DETAIL)
    await expect(firstRun(page).getByTestId('first-run-create-kind')).toBeVisible()
    await expectSettled(page)
  })

  test('empty kubeconfig: says no clusters are configured', async ({ page }) => {
    await open(page, 'kubeconfig-empty')
    await expect(firstRun(page)).toContainText('No clusters configured')
    await expect(firstRun(page).getByTestId('first-run-create-kind')).toBeVisible()
    await expectSettled(page)
  })

  test('no current context: lists the contexts and picking one browses it', async ({ page }) => {
    await open(page, 'no-current-context')
    await expect(firstRun(page)).toHaveAttribute('data-first-run', 'pick')
    await expect(firstRun(page)).toContainText('Pick a context')
    await expectSettled(page)
    await firstRun(page).getByTestId('first-run-context').filter({ hasText: 'kind-b' }).click()
    await expectRowCount(page, PODS_KIND_B.length)
  })

  test('unreachable cluster: explains it, retries, and switching to a working context recovers', async ({
    page,
  }) => {
    await open(page, 'context-unreachable')
    await expect(firstRun(page)).toHaveAttribute('data-first-run', 'connection')
    await expect(firstRun(page)).toContainText('Cannot reach kind-a')
    await expect(firstRun(page)).toContainText(UNREACHABLE_SERVER)
    await expectSettled(page)
    // The verbatim error is one click away, not the headline.
    await expect(firstRun(page).getByTestId('first-run-detail')).toBeHidden()
    await firstRun(page).getByText('Details').click()
    await expect(firstRun(page).getByTestId('first-run-detail')).toContainText(UNREACHABLE_DETAIL)

    await clearSent(page)
    await firstRun(page).getByTestId('first-run-retry').click()
    await expect
      .poll(async () => (await sent(page)).filter((c) => c.type === 'SetContext'))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-a' } }])
    // Still down: the same explained state comes back, not a skeleton.
    await expect(firstRun(page)).toContainText('Cannot reach kind-a')
    await expectSettled(page)

    await firstRun(page).getByTestId('first-run-context').filter({ hasText: 'kind-b' }).click()
    await expectRowCount(page, PODS_KIND_B.length)
    await expect(listPane(page).getByTestId('first-run')).toHaveCount(0)
  })

  test('rejected credentials: says so instead of showing Connected', async ({ page }) => {
    await open(page, 'context-unauthorized')
    await expect(firstRun(page)).toContainText('kind-a rejected the credentials')
    await expect(firstRun(page)).toContainText('401')
    await expectSettled(page)
  })

  test('a cluster lost mid-session shows the explained state, then recovers', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
    await emit(
      page,
      batch([
        system(700, {
          type: 'ActiveConnectionProblem',
          data: {
            context: 'kind-a',
            problem: {
              kind: 'Unreachable',
              server: UNREACHABLE_SERVER,
              detail: UNREACHABLE_DETAIL,
            },
          },
        }),
      ]),
    )
    await expect(firstRun(page)).toContainText('Cannot reach kind-a')
    await emit(
      page,
      batch([
        system(701, {
          type: 'ActiveConnectionProblem',
          data: { context: 'kind-a', problem: null },
        }),
      ]),
    )
    await expectRowCount(page, 500)
  })
})

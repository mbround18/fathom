/**
 * T020 (US3 Phase 5): local port picker with conflict detection. Covers spec.md US3 Acceptance
 * Scenarios 1-4: the field is pre-filled with a suggested port that has already been checked;
 * editing to a bound port dispatches a debounced `CheckPortAvailability` and shows a conflict
 * warning that blocks confirming; changing to a free port clears the warning; confirming despite
 * a race (unresolved conflict UI, backend actually fails) still surfaces the verbatim backend
 * error rather than a false success. All new `data-testid`s are prefixed `port-forward-` for
 * isolation from sibling 007/008/010 specs.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, list, nsRoute, read } from '../fixtures/builders'
import {
  clearSent,
  confirmDialog,
  emit,
  finish,
  inspector,
  navigateTo,
  nextRequestId,
  row,
  sentTypes,
  open as openApp,
} from './helpers'

const POD = {
  name: 'web-1',
  namespace: 'default',
  phase: 'Running',
  node: 'kind-a-worker',
  ready: 1,
  total: 1,
  restarts: 0,
}

const POD_DETAILS = {
  namespace: POD.namespace,
  name: POD.name,
  creation_timestamp: '2026-09-20T08:15:00Z',
  labels: {},
  annotations: {},
  containers: ['web'],
  ports: [{ container: 'web', name: 'http', port: 8080, protocol: 'TCP' }],
  resource_specs: [],
  owner: null,
  resolved_owner: null,
  persistent_volume_claims: [],
  raw_json: '{}',
}

async function openPodForwardDialog(page: Page) {
  await openApp(page)
  await navigateTo(page, 'Pods', 'Workloads')
  await expect.poll(() => sentTypes(page)).toContain('RefreshPods')
  await page.waitForTimeout(600)
  await clearSent(page)
  await emit(page, batch([list(1, nsRoute('kind-a', 'workloads/pods'), { type: 'PodsUpdated', data: [POD] })]))
  await row(page, `${POD.namespace}/${POD.name}`, 'workloads/pods').click()
  await expect.poll(() => sentTypes(page)).toContain('SelectPod')
  await emit(
    page,
    batch([
      read(2, nsRoute('kind-a', 'workloads/pods', POD.namespace), {
        type: 'PodDetailsUpdated',
        data: POD_DETAILS,
      }),
    ]),
  )
  const insp = inspector(page)
  await insp.getByTestId('port-forward-tab').click()
  await clearSent(page)
  await insp.getByTestId('port-forward-action-web-8080').click()
  const dialog = confirmDialog(page)
  await expect(dialog).toBeVisible()
  return dialog
}

test.describe('US3 port forward port picker', () => {
  test('Scenario 1: the field is pre-filled with a suggested port already checked for availability', async ({
    page,
  }) => {
    const dialog = await openPodForwardDialog(page)
    const field = dialog.getByTestId('port-forward-local-port')
    await expect(field).toHaveValue('8080')
    await expect.poll(() => sentTypes(page)).toContain('CheckPortAvailability')
    const calls = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    expect(calls.find((c) => c.type === 'CheckPortAvailability')).toEqual({
      type: 'CheckPortAvailability',
      data: { port: 8080 },
    })
  })

  test('Scenario 2/3: editing to a bound port warns and blocks confirm; a free port clears it', async ({
    page,
  }) => {
    const dialog = await openPodForwardDialog(page)
    const field = dialog.getByTestId('port-forward-local-port')
    await clearSent(page)

    await field.fill('9999')
    await expect.poll(() => sentTypes(page), { timeout: 2000 }).toContain('CheckPortAvailability')
    const calls = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    expect(calls.find((c) => c.type === 'CheckPortAvailability')).toEqual({
      type: 'CheckPortAvailability',
      data: { port: 9999 },
    })

    await emit(
      page,
      batch([
        {
          version: 1,
          seq: 10,
          route: globalRoute('system'),
          correlation: { type: 'None' },
          event: { type: 'System', data: { type: 'PortAvailability', data: { port: 9999, available: false } } },
        },
      ]),
    )
    await expect(dialog.getByTestId('port-forward-port-conflict')).toBeVisible()
    await expect(dialog.getByTestId('port-forward-confirm-blocked')).toBeVisible()

    // Confirm is blocked while the conflict is unresolved: clicking it dispatches nothing.
    await clearSent(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect(dialog).toBeVisible()
    expect(await sentTypes(page)).not.toContain('StartPortForward')

    // Switching to a free port clears the warning and unblocks confirm.
    await field.fill('9998')
    await emit(
      page,
      batch([
        {
          version: 1,
          seq: 11,
          route: globalRoute('system'),
          correlation: { type: 'None' },
          event: { type: 'System', data: { type: 'PortAvailability', data: { port: 9998, available: true } } },
        },
      ]),
    )
    await expect(dialog.getByTestId('port-forward-port-conflict')).toBeHidden()
    await expect(dialog.getByTestId('port-forward-confirm-blocked')).toBeHidden()

    await clearSent(page)
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')
    await finish(page, requestId, true)
    await expect(dialog).toBeHidden()
  })

  test('Scenario 4: confirming despite a race, a late backend bind failure is shown verbatim', async ({
    page,
  }) => {
    const dialog = await openPodForwardDialog(page)
    await clearSent(page)
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')
    await finish(page, requestId, false, 'bind: address already in use')
    await expect(dialog.getByTestId('port-forward-start-error')).toContainText('address already in use')
    await expect(dialog).toBeVisible()
  })
})

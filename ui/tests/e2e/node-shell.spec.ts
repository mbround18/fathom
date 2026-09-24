/**
 * 037-node-shell: the Node Shell tab against the fake backend. The fake replays nothing for
 * these commands, so each test injects the backend's events and asserts the commands the UI
 * sends (start, attach via StartPodExec, and StopNodeShell on every UI-side exit path).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, envelope, globalRoute, list } from '../fixtures/builders'
import type { NodeShellPhase, NodeShellAccessCheck, SystemEvent } from '../../src/protocol/types'
import {
  clearSent,
  confirmDialog,
  emit,
  inspector,
  open,
  sent,
  sentTypes,
  expectNoHorizontalScroll,
} from './helpers'

const NODE = {
  name: 'kind-a-worker',
  status: 'Ready',
  role: 'worker',
  pod_count: 8,
  pod_capacity: 110,
  cpu_allocatable_millicores: 2000,
  memory_allocatable_bytes: 4_000_000_000,
}
const NODES_ROUTE = clusterRoute('kind-a', 'nodes')
const POD = 'fathom-node-shell-abc123-x1y2z'

let seq = 5000
const sys = (route: ReturnType<typeof clusterRoute>, data: SystemEvent) =>
  envelope(++seq, route, { type: 'System', data })

const settingsEvent = () =>
  sys(globalRoute(), {
    type: 'NodeShellSettingsLoaded',
    data: {
      context: 'kind-a',
      settings: {
        image: 'busybox:stable',
        namespace: 'default',
        image_pull_secret: null,
        deadline_seconds: 14400,
      },
    },
  })

const allowed = (verb: string, resource: string, sub: string | null = null) =>
  ({
    verb,
    resource,
    subresource: sub,
    namespace: 'default',
    outcome: { type: 'Allowed' },
  }) satisfies NodeShellAccessCheck

const accessEvent = (checks: NodeShellAccessCheck[]) =>
  sys(NODES_ROUTE, { type: 'NodeShellAccessChecked', data: { namespace: 'default', checks } })

const ALL_ALLOWED = [
  allowed('create', 'pods'),
  allowed('delete', 'pods'),
  allowed('get', 'pods'),
  allowed('create', 'pods', 'exec'),
  allowed('get', 'nodes'),
]

const statusEvent = (
  phase: NodeShellPhase,
  extra: { detail?: string | null; leftover?: boolean; session_id?: number } = {},
) =>
  sys(NODES_ROUTE, {
    type: 'NodeShellStatus',
    data: {
      session_id: extra.session_id ?? 1,
      phase,
      node: NODE.name,
      namespace: 'default',
      pod: POD,
      detail: extra.detail ?? null,
      leftover: extra.leftover ?? false,
    },
  })

async function openShellTab(page: Page, checks: NodeShellAccessCheck[] = ALL_ALLOWED) {
  await open(page)
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open navigator' }).click()
  }
  const toggle = nav.getByRole('button', { name: 'Overview', expanded: false })
  if (await toggle.isVisible()) await toggle.click()
  await nav.locator('[data-section="nodes"]').click()
  await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['RefreshNodes']))
  await emit(page, batch([list(1, NODES_ROUTE, { type: 'NodesUpdated', data: [NODE] })]))
  await page.locator(`[role="row"][data-row-key="${NODE.name}"]`).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await expect.poll(() => sentTypes(page)).toContain('SaveUiConfig')
  await clearSent(page)
  await insp.getByRole('tab', { name: 'Shell' }).click()
  await expect
    .poll(() => sentTypes(page))
    .toEqual(expect.arrayContaining(['GetNodeShellSettings', 'CheckNodeShellAccess']))
  await emit(page, batch([settingsEvent(), accessEvent(checks)]))
  return insp
}

test.describe('037 node shell', () => {
  test('confirm, create, attach, exit: StopNodeShell on shell exit', async ({ page }) => {
    const insp = await openShellTab(page)
    await clearSent(page)

    await insp.getByTestId('node-shell-start').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    // The dialog names the node and states the privileges granted.
    await expect(dialog).toContainText(NODE.name)
    await expect(dialog).toContainText('busybox:stable')
    await expect(dialog).toContainText('privileged')
    await expect(dialog).toContainText('/host')
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'StartNodeShell',
          data: {
            node: NODE.name,
            namespace: 'default',
            image: 'busybox:stable',
            image_pull_secret: null,
            deadline_seconds: 14400,
          },
        },
      ])

    await emit(page, batch([statusEvent('Creating'), statusEvent('Pending')]))
    await expect(insp.getByTestId('node-shell-status')).toHaveAttribute('data-phase', 'Pending')
    // No terminal until the pod is Running.
    await expect(insp.getByTestId('node-shell-xterm')).toHaveCount(0)

    await clearSent(page)
    await emit(page, batch([statusEvent('Running')]))
    await expect(insp.getByTestId('node-shell-xterm')).toBeVisible()
    await expect
      .poll(() => sent(page))
      .toContainEqual({
        type: 'StartPodExec',
        data: { namespace: 'default', name: POD, container: 'shell' },
      })

    await emit(page, batch([statusEvent('Attached')]))
    await expect(insp.getByTestId('node-shell-status')).toHaveAttribute('data-phase', 'Attached')
    await expectNoHorizontalScroll(page)

    // The shell exits (stream closed): the UI asks the backend to delete the pod.
    await clearSent(page)
    await emit(
      page,
      batch([
        envelope(
          ++seq,
          {
            section: 'workloads/pods',
            scope: 'Namespaced',
            context: 'kind-a',
            namespace: 'default',
          },
          {
            type: 'Stream',
            data: { type: 'StreamClosed', data: { stream_id: 1, reason: 'exit' } },
          },
        ),
      ]),
    )
    await expect
      .poll(() => sent(page))
      .toContainEqual({ type: 'StopNodeShell', data: { session_id: 1, namespace: 'default' } })

    await emit(page, batch([statusEvent('Terminating'), statusEvent('Removed')]))
    await expect(insp.getByTestId('node-shell-status')).toHaveAttribute('data-phase', 'Removed')
  })

  test('closing the tab while a pod is running sends StopNodeShell', async ({ page }) => {
    const insp = await openShellTab(page)
    await emit(page, batch([statusEvent('Creating'), statusEvent('Pending')]))
    await expect(insp.getByTestId('node-shell-status')).toBeVisible()
    await clearSent(page)
    await insp.getByRole('tab', { name: 'Actions' }).click()
    await expect
      .poll(() => sent(page))
      .toContainEqual({ type: 'StopNodeShell', data: { session_id: 1, namespace: 'default' } })
  })

  test('missing permissions disable the action and list what is missing, without a dialog', async ({
    page,
  }) => {
    const insp = await openShellTab(page, [
      allowed('create', 'pods'),
      {
        verb: 'create',
        resource: 'pods',
        subresource: 'exec',
        namespace: 'default',
        outcome: { type: 'Denied', data: { reason: 'exec is forbidden' } },
      },
    ])
    const denied = insp.getByTestId('node-shell-denied')
    await expect(denied).toContainText('create pods/exec: exec is forbidden')
    await expect(insp.getByTestId('node-shell-start')).toBeDisabled()
    await expect(confirmDialog(page)).toHaveCount(0)
  })

  test('a failed start shows the API error verbatim; a leftover pod can be retried', async ({
    page,
  }) => {
    const insp = await openShellTab(page)
    await emit(
      page,
      batch([
        statusEvent('Creating'),
        statusEvent('Failed', {
          detail: 'pods "x" is forbidden: violates PodSecurity "restricted:latest"',
        }),
        statusEvent('Terminating'),
        statusEvent('Failed', { detail: 'delete refused: forbidden', leftover: true }),
      ]),
    )
    await expect(insp.getByTestId('node-shell-detail')).toContainText('delete refused: forbidden')
    await expect(insp.getByTestId('node-shell-leftover')).toContainText(POD)
    await clearSent(page)
    await insp.getByTestId('node-shell-stop').click()
    await expect
      .poll(() => sent(page))
      .toContainEqual({ type: 'StopNodeShell', data: { session_id: 1, namespace: 'default' } })
  })

  test('the pending detail (conditions) is shown and Cancel stops the session', async ({
    page,
  }) => {
    const insp = await openShellTab(page)
    await emit(
      page,
      batch([
        statusEvent('Pending', { detail: 'ImagePullBackOff: Back-off pulling image "nope"' }),
      ]),
    )
    await expect(insp.getByTestId('node-shell-detail')).toContainText('ImagePullBackOff')
    await clearSent(page)
    await insp.getByTestId('node-shell-stop').click()
    await expect
      .poll(() => sent(page))
      .toContainEqual({ type: 'StopNodeShell', data: { session_id: 1, namespace: 'default' } })
  })

  test('orphan sweep lists leftover pods and deletes only after per-pod confirmation', async ({
    page,
  }) => {
    const insp = await openShellTab(page)
    await clearSent(page)
    await insp.getByTestId('node-shell-find-orphans').click()
    await expect
      .poll(() => sent(page))
      .toContainEqual({ type: 'ListNodeShellOrphans', data: { namespace: null } })
    await emit(
      page,
      batch([
        sys(NODES_ROUTE, {
          type: 'NodeShellOrphans',
          data: {
            pods: [
              {
                namespace: 'default',
                name: 'fathom-node-shell-old-11111',
                node: NODE.name,
                phase: 'Failed',
                created_at: null,
              },
            ],
          },
        }),
      ]),
    )
    await expect(insp.getByTestId('node-shell-orphan')).toContainText('fathom-node-shell-old-11111')
    await clearSent(page)
    await insp.getByTestId('node-shell-orphan-delete').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toContainText('fathom-node-shell-old-11111')
    // Cancel sends nothing.
    await dialog.getByTestId('confirm-cancel').click()
    expect(await sent(page)).toEqual([])
    await insp.getByTestId('node-shell-orphan-delete').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'DeleteNodeShellOrphans',
          data: { pods: [{ namespace: 'default', name: 'fathom-node-shell-old-11111' }] },
        },
      ])
  })
})

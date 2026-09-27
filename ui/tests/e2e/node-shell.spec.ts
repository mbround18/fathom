/**
 * 037-node-shell: the Node Shell tab against the fake backend. The fake replays nothing for
 * these commands, so each test injects the backend's events and asserts the commands the UI
 * sends (start, attach via StartPodExec, and StopNodeShell on every UI-side exit path).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, envelope, globalRoute, list, system } from '../fixtures/builders'
import type { NodeShellPhase, NodeShellAccessCheck, SystemEvent } from '../../src/protocol/types'
import {
  clearSent,
  confirmDialog,
  emit,
  inspector,
  open,
  sentIgnoringNodePoll,
  sentTypes,
  expectNoHorizontalScroll,
  navigateTo,
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
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  const toggle = nav.getByRole('button', { name: 'Cluster', expanded: false })
  if (await toggle.isVisible()) await toggle.click()
  await nav.locator('[data-section="nodes"]').click()
  await expect.poll(() => sentTypes(page)).toEqual(expect.arrayContaining(['RefreshNodes']))
  await emit(page, batch([list(1, NODES_ROUTE, { type: 'NodesUpdated', data: [NODE] })]))
  await page.locator(`[role="row"][data-row-key="${NODE.name}"]`).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await expect.poll(() => sentTypes(page, true)).toContain('SaveUiConfig')
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
      .poll(() => sentIgnoringNodePoll(page))
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
      .poll(() => sentIgnoringNodePoll(page))
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
      .poll(() => sentIgnoringNodePoll(page))
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
      .poll(() => sentIgnoringNodePoll(page))
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
      .poll(() => sentIgnoringNodePoll(page))
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
      .poll(() => sentIgnoringNodePoll(page))
      .toContainEqual({ type: 'StopNodeShell', data: { session_id: 1, namespace: 'default' } })
  })

  test('orphan sweep lists leftover pods and deletes only after per-pod confirmation', async ({
    page,
  }) => {
    const insp = await openShellTab(page)
    await clearSent(page)
    await insp.getByTestId('node-shell-find-orphans').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
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
    expect(await sentIgnoringNodePoll(page)).toEqual([])
    await insp.getByTestId('node-shell-orphan-delete').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        {
          type: 'DeleteNodeShellOrphans',
          data: { pods: [{ namespace: 'default', name: 'fathom-node-shell-old-11111' }] },
        },
      ])
  })

  test('a session override applies to one start without changing the saved defaults', async ({
    page,
  }) => {
    const insp = await openShellTab(page)
    await insp.getByText('Session options').click()
    await insp.getByTestId('node-shell-image').fill('alpine:3.20')
    await insp.getByTestId('node-shell-deadline').fill('10')
    await clearSent(page)
    await insp.getByTestId('node-shell-start').click()
    await expect(confirmDialog(page)).toContainText('alpine:3.20')
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        {
          type: 'StartNodeShell',
          data: {
            node: NODE.name,
            namespace: 'default',
            image: 'alpine:3.20',
            image_pull_secret: null,
            deadline_seconds: 600,
          },
        },
      ])
    expect(await sentTypes(page)).not.toContain('SetNodeShellSettings')
  })

  test('keyboard: the start action opens the dialog with Enter and Escape cancels', async ({
    page,
  }) => {
    const insp = await openShellTab(page)
    await clearSent(page)
    const start = insp.getByTestId('node-shell-start')
    await start.focus()
    await expect(start).toBeFocused()
    await page.keyboard.press('Enter')
    await expect(confirmDialog(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(confirmDialog(page)).toHaveCount(0)
    expect(await sentIgnoringNodePoll(page)).toEqual([])
    // Session options are a native disclosure, operable from the keyboard.
    const summary = insp.getByText('Session options')
    await summary.focus()
    await page.keyboard.press('Enter')
    await expect(insp.getByTestId('node-shell-image')).toBeVisible()
    await expectNoHorizontalScroll(page)
  })

  test('status and denial regions are announced, controls are labelled, reduced motion holds', async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' })
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
    await expect(insp.getByRole('alert').filter({ hasText: 'missing permissions' })).toBeVisible()
    await expect(insp.getByTestId('node-shell-start')).toHaveAccessibleName(/open node shell/i)
    await insp.getByText('Session options').click()
    await expect(insp.getByLabel('Image', { exact: true })).toBeVisible()
    await expect(insp.getByLabel('Auto-delete after (minutes)')).toBeVisible()
    // No running CSS animation or transition on the panel under reduced motion.
    const animated = await insp.getByTestId('node-shell-panel').evaluate(
      (el) =>
        [el, ...el.querySelectorAll('*')].filter((n) => {
          const cs = getComputedStyle(n)
          return cs.animationName !== 'none' && parseFloat(cs.animationDuration) > 0.01
        }).length,
    )
    expect(animated).toBe(0)
    await expectNoHorizontalScroll(page)
  })
})

const SETTINGS_CTX = 'kind-a'
const BASE_CLUSTER_SETTINGS = {
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

async function goToNodeShellSettings(page: Page) {
  await open(page)
  await navigateTo(page, 'Cluster Settings', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
  await emit(
    page,
    batch([
      system(1, {
        type: 'ClusterSettingsLoaded',
        data: { context: SETTINGS_CTX, settings: BASE_CLUSTER_SETTINGS },
      }),
    ]),
  )
  await expect.poll(() => sentTypes(page)).toContain('GetNodeShellSettings')
  await emit(page, batch([settingsEvent()]))
  return page.getByTestId('node-shell-settings')
}

test.describe('037 node shell settings persistence', () => {
  test('saved defaults are sent, normalized values shown, and survive a restart', async ({
    page,
  }) => {
    const section = await goToNodeShellSettings(page)
    await expect(section.getByTestId('node-shell-settings-image')).toHaveValue('busybox:stable')
    await expect(section.getByTestId('node-shell-settings-deadline')).toHaveValue('240')

    await section.getByTestId('node-shell-settings-image').fill('ghcr.io/acme/debug:1')
    await section.getByTestId('node-shell-settings-namespace').fill('ops')
    await section.getByTestId('node-shell-settings-deadline').fill('30')
    await clearSent(page)
    await section.getByTestId('node-shell-settings-save').click()
    await expect
      .poll(() => sentIgnoringNodePoll(page))
      .toEqual([
        {
          type: 'SetNodeShellSettings',
          data: {
            settings: {
              image: 'ghcr.io/acme/debug:1',
              namespace: 'ops',
              image_pull_secret: null,
              deadline_seconds: 1800,
            },
          },
        },
      ])

    // The backend replies with the normalized values it persisted.
    const saved = {
      image: 'ghcr.io/acme/debug:1',
      namespace: 'ops',
      image_pull_secret: null,
      deadline_seconds: 1800,
    }
    const loaded = (n: number) =>
      system(n, {
        type: 'NodeShellSettingsLoaded',
        data: { context: SETTINGS_CTX, settings: saved },
      })
    await emit(page, batch([loaded(60)]))
    await expect(section.getByTestId('node-shell-settings-namespace')).toHaveValue('ops')

    // Restart: fresh page and store; the backend replays what it read from node-shell.json.
    await open(page)
    await navigateTo(page, 'Cluster Settings', 'Team')
    await emit(
      page,
      batch([
        system(1, {
          type: 'ClusterSettingsLoaded',
          data: { context: SETTINGS_CTX, settings: BASE_CLUSTER_SETTINGS },
        }),
        loaded(61),
      ]),
    )
    const again = page.getByTestId('node-shell-settings')
    await expect(again.getByTestId('node-shell-settings-image')).toHaveValue('ghcr.io/acme/debug:1')
    await expect(again.getByTestId('node-shell-settings-deadline')).toHaveValue('30')
    await expectNoHorizontalScroll(page)
  })

  test('an invalid deadline blocks saving', async ({ page }) => {
    const section = await goToNodeShellSettings(page)
    await section.getByTestId('node-shell-settings-deadline').fill('2')
    await expect(section.getByTestId('node-shell-settings-save')).toBeDisabled()
    await expect(section.getByRole('alert')).toContainText('at least 5 minutes')
  })
})

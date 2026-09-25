/**
 * T010 (US1 Phase 3): starting a port forward from a resource inspector's "Forward" tab.
 * Covers spec.md US1 Acceptance Scenarios 1-5: a Pod-port forward names namespace/pod/container/
 * port in the confirm dialog; a Service-port forward resolves a backing pod and names it;
 * confirming dispatches `StartPortForward` and the row goes `Connecting` -> `Active` via
 * `PortForwardsUpdated`; cancelling dispatches nothing; a backend error is shown verbatim with no
 * misleading active row. Self-contained fixtures (not `PODS_500`/`gen-pods.ts`), per the
 * `us2-node-actions.spec.ts`/`us3-drilldown-backstack.spec.ts` precedent. All new `data-testid`s
 * are prefixed `port-forward-` for isolation from sibling 007/008/010 specs.
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
  sent,
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
  containers_lifecycle: [],
  owner: null,
  resolved_owner: null,
  persistent_volume_claims: [],
  raw_json: '{}',
}

const SERVICE = {
  name: 'web-svc',
  namespace: 'default',
  service_type: 'ClusterIP',
  cluster_ip: '10.0.0.5',
  external_ips: [],
  port_count: 1,
}

const SERVICE_DETAILS = {
  namespace: SERVICE.namespace,
  name: SERVICE.name,
  creation_timestamp: '2026-09-20T08:15:00Z',
  labels: {},
  annotations: {},
  service_type: SERVICE.service_type,
  cluster_ip: SERVICE.cluster_ip,
  cluster_ips: [SERVICE.cluster_ip],
  external_ips: [],
  external_name: null,
  selector: { app: 'web' },
  ports: [{ name: 'http', port: 80, target_port: '8080', protocol: 'TCP', node_port: null }],
  session_affinity: 'None',
  load_balancer_ip: null,
  load_balancer_ingress_hosts: [],
  raw_json: '{}',
}

const SERVICE_POD_NAME = 'web-svc-abcd'

async function openPodForwardTab(page: Page): Promise<void> {
  await openApp(page)
  await navigateTo(page, 'Pods', 'Workloads')
  await expect.poll(() => sentTypes(page)).toContain('RefreshPods')
  // Expanding a nav group (if not already expanded) schedules a debounced `SaveUiConfig`
  // (500ms). Let any such debounce elapse and clear it now so it cannot later steal a request
  // id from a `nextRequestId()`/submit-click pair (both `send` calls share one counter in the
  // fake transport). Not asserted, since the group may already be expanded and no save fires.
  await page.waitForTimeout(600)
  await clearSent(page)
  await emit(
    page,
    batch([list(1, nsRoute('kind-a', 'workloads/pods'), { type: 'PodsUpdated', data: [POD] })]),
  )
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
}

test.describe('US1 port forward start', () => {
  test('Scenario 1/3: a Pod-port forward names namespace/pod/container/port and starts a row', async ({
    page,
  }) => {
    await openPodForwardTab(page)
    const insp = inspector(page)
    await clearSent(page)

    await insp.getByTestId('port-forward-action-web-8080').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    const detail = dialog.getByTestId('port-forward-start-dialog')
    await expect(detail).toContainText('web')
    await expect(detail).toContainText(POD.namespace)
    await expect(detail).toContainText(POD.name)
    await expect(detail).toContainText('8080')

    // The dialog dispatches its own pre-check on open (US3 Scenario 1); clear it. A debounced
    // nav-expand `SaveUiConfig` can still land after this point, so assert on the
    // StartPortForward call specifically rather than requiring `sent(page)` to equal exactly
    // one entry.
    await expect.poll(() => sentTypes(page)).toContain('CheckPortAvailability')
    await clearSent(page)
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')
    const startCalls = await sent(page)
    expect(startCalls.find((c) => c.type === 'StartPortForward')).toEqual({
      type: 'StartPortForward',
      data: {
        namespace: POD.namespace,
        pod: POD.name,
        local_port: 8080,
        remote_port: 8080,
        container: 'web',
      },
    })

    await finish(page, requestId, true)
    await expect(page.getByTestId('port-forward-start-toast').first()).toBeVisible()
    await expect(dialog).toBeHidden()

    // FR-010/SC-006: the row goes Connecting -> Active strictly via PortForwardsUpdated, never
    // anticipated client-side before the backend confirms it.
    await navigateTo(page, 'Port Forwards', 'Network')
    await emit(
      page,
      batch([
        list(3, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [
            {
              id: 'pf-1',
              namespace: POD.namespace,
              pod: POD.name,
              local_port: 8080,
              remote_port: 8080,
              container: 'web',
              status: 'Connecting',
              auto_start: false,
            },
          ],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    await expect(forwardRow).toContainText('Connecting')

    await emit(
      page,
      batch([
        list(4, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [
            {
              id: 'pf-1',
              namespace: POD.namespace,
              pod: POD.name,
              local_port: 8080,
              remote_port: 8080,
              container: 'web',
              status: 'Active',
              auto_start: false,
            },
          ],
        }),
      ]),
    )
    await expect(forwardRow).toContainText('Active')
  })

  test('Scenario 2: a Service-port forward resolves a backing pod and names it', async ({
    page,
  }) => {
    await openApp(page)
    await navigateTo(page, 'Services', 'Network')
    await expect.poll(() => sentTypes(page)).toContain('RefreshServices')
    // Expanding a nav group (if not already expanded) schedules a debounced `SaveUiConfig`
    // (500ms). Let any such debounce elapse and clear it now so it cannot later steal a request
    // id from a `nextRequestId()`/submit-click pair (both `send` calls share one counter in the
    // fake transport). Not asserted, since the group may already be expanded and no save fires.
    await page.waitForTimeout(600)
    await clearSent(page)
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', 'network/services'), {
          type: 'ServicesUpdated',
          data: [SERVICE],
        }),
      ]),
    )
    await row(page, `${SERVICE.namespace}/${SERVICE.name}`, 'network/services').click()
    await expect.poll(() => sentTypes(page)).toContain('SelectService')
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', 'network/services', SERVICE.namespace), {
          type: 'ServiceDetailsUpdated',
          data: SERVICE_DETAILS,
        }),
      ]),
    )

    const insp = inspector(page)
    await clearSent(page)
    await insp.getByTestId('port-forward-tab').click()
    await expect.poll(() => sentTypes(page)).toContain('RefreshRelatedResources')
    await emit(
      page,
      batch([
        read(3, nsRoute('kind-a', 'network/services', SERVICE.namespace), {
          type: 'RelatedResourcesUpdated',
          data: {
            section: 'network/services',
            kind: 'Service',
            namespace: SERVICE.namespace,
            name: SERVICE.name,
            related: [
              {
                section: 'workloads/pods',
                kind: 'Pod',
                namespace: SERVICE.namespace,
                name: SERVICE_POD_NAME,
                relation: 'selects',
                status_hint: 'Running',
              },
            ],
          },
        }),
      ]),
    )

    await clearSent(page)
    await insp.getByTestId('port-forward-action-service-80').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    const detail = dialog.getByTestId('port-forward-start-dialog')
    await expect(detail).toContainText(SERVICE_POD_NAME)
    await expect(detail).toContainText(SERVICE.name)

    await expect.poll(() => sentTypes(page)).toContain('CheckPortAvailability')
    // A debounced nav-group-expand `SaveUiConfig` (from expanding "Network" above) can still
    // land after this point, so assert on the StartPortForward call specifically rather than
    // requiring `sent(page)` to equal exactly one entry.
    await clearSent(page)
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')
    const calls = await sent(page)
    expect(calls.find((c) => c.type === 'StartPortForward')).toEqual({
      type: 'StartPortForward',
      data: {
        namespace: SERVICE.namespace,
        pod: SERVICE_POD_NAME,
        local_port: 80,
        remote_port: 80,
        container: null,
      },
    })
    await finish(page, requestId, true)
    await expect(dialog).toBeHidden()
  })

  test('Scenario 4: cancelling the dialog dispatches nothing', async ({ page }) => {
    await openPodForwardTab(page)
    const insp = inspector(page)
    await clearSent(page)

    await insp.getByTestId('port-forward-action-web-8080').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    // The dialog's own availability pre-check (US3 Scenario 1) is a read, not a mutation; only
    // that is sent before Cancel — no StartPortForward.
    await expect.poll(() => sentTypes(page)).toContain('CheckPortAvailability')
    await clearSent(page)
    await dialog.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()
    expect(await sent(page)).toEqual([])
  })

  test('Scenario 5: a backend start failure is shown verbatim with no misleading active row', async ({
    page,
  }) => {
    await openPodForwardTab(page)
    const insp = inspector(page)
    await clearSent(page)

    await insp.getByTestId('port-forward-action-web-8080').click()
    const dialog = confirmDialog(page)
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')

    await finish(page, requestId, false, 'bind: address already in use')
    await expect(dialog.getByTestId('port-forward-start-error')).toContainText(
      'address already in use',
    )
    // The dialog stays open (no misleading close-as-success) and no row was ever anticipated.
    await expect(dialog).toBeVisible()

    await dialog.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()
    await navigateTo(page, 'Port Forwards', 'Network')
    await expect(page.getByTestId('port-forward-row')).toHaveCount(0)
  })
})

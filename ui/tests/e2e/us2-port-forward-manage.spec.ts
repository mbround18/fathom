/**
 * T015 (US2 Phase 4): managing the live list of active forwards. Covers spec.md US2 Acceptance
 * Scenarios 1-5 (Scenario 6, app-exit, is explicitly a no-op per spec.md's Assumptions — the
 * backend already tears down on exit and the UI adds no extra confirmation, so there is nothing
 * to assert e2e): every row shows local port/target/origin/status; Stop dispatches
 * `StopPortForward` and only reaches `Closed` after the backend confirms; Restart on an
 * `Error`/`Closed` row reuses namespace/pod/port through the same confirm+port-check flow; an
 * out-of-band `PortForwardTunnel` stream event updates a row live with no user action; a fresh
 * `PortForwardsUpdated` (as after a context switch) fully replaces the rendered rows. All new
 * `data-testid`s are prefixed `port-forward-` for isolation from sibling 007/008/010 specs.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, list, nsRoute, read, stream } from '../fixtures/builders'
import type { PortForward } from '../../src/protocol/types'
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

function pf(overrides: Partial<PortForward> = {}): PortForward {
  return {
    id: 'pf-1',
    namespace: 'default',
    pod: 'web-1',
    local_port: 8080,
    remote_port: 8080,
    container: 'web',
    status: 'Active',
    auto_start: false,
    ...overrides,
  }
}

async function openPortForwardsPanel(page: Page): Promise<void> {
  await openApp(page)
  await navigateTo(page, 'Port Forwards', 'Network')
  await expect.poll(() => sentTypes(page)).toContain('RefreshPortForwards')
  // Expanding "Network" (if not already expanded) schedules a debounced `SaveUiConfig` (500ms).
  // Let any such debounce elapse and clear it now so it cannot later steal a request id, or
  // pollute a `sentTypes` snapshot, from an action dispatched right after this helper returns
  // (both `send` calls share one counter in the fake transport).
  await page.waitForTimeout(600)
  await clearSent(page)
}

test.describe('US2 port forward manage', () => {
  test('Scenario 1: every row shows local port, target, originating service, and status', async ({
    page,
  }) => {
    // Establish an origin tag the way a real operator would: launch from the Service inspector
    // (same flow as US1 Scenario 2), then observe the tag rendered in the list.
    await openApp(page)
    await navigateTo(page, 'Services', 'Network')
    await expect.poll(() => sentTypes(page)).toContain('RefreshServices')
    await page.waitForTimeout(600)
    await clearSent(page)
    await emit(
      page,
      batch([list(1, nsRoute('kind-a', 'network/services'), { type: 'ServicesUpdated', data: [SERVICE] })]),
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
    await insp.getByTestId('port-forward-action-service-80').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('CheckPortAvailability')
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')
    await finish(page, requestId, true)
    await expect(dialog).toBeHidden()

    await navigateTo(page, 'Port Forwards', 'Network')
    await emit(
      page,
      batch([
        list(4, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ id: 'pf-svc', namespace: SERVICE.namespace, pod: SERVICE_POD_NAME, local_port: 80, remote_port: 80, container: null, status: 'Active' })],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    await expect(forwardRow).toHaveCount(1)
    await expect(forwardRow.getByTestId('port-forward-local-port')).toHaveText('80')
    await expect(forwardRow.getByTestId('port-forward-target')).toContainText(
      `${SERVICE.namespace}/${SERVICE_POD_NAME}:80`,
    )
    await expect(forwardRow.getByTestId('port-forward-origin-service')).toContainText(SERVICE.name)
    await expect(forwardRow).toContainText('Active')
  })

  test('Scenario 2: Stop dispatches StopPortForward and the row stays non-Closed until confirmed', async ({
    page,
  }) => {
    await openPortForwardsPanel(page)
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ status: 'Active' })],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    await expect(forwardRow).toContainText('Active')

    await clearSent(page)
    await forwardRow.getByTestId('port-forward-stop').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('port-forward-stop-detail')).toContainText('8080')
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toEqual(['StopPortForward'])

    // Not closed yet: the row still reflects the backend's last-known status until an explicit
    // PortForwardTunnel/PortForwardsUpdated event says otherwise (FR-010/SC-006 for stop, same
    // rule as start).
    await expect(forwardRow).toContainText('Active')

    await emit(
      page,
      batch([
        stream(2, globalRoute('network/portforwards'), {
          type: 'PortForwardTunnel',
          data: { tunnel_id: 'pf-1', status: 'Closed', local_endpoint: null },
        }),
      ]),
    )
    await expect(forwardRow).toContainText('Closed')
  })

  test('Scenario 3: Restart on an Error row reuses namespace/pod/port through confirm+port-check', async ({
    page,
  }) => {
    await openPortForwardsPanel(page)
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ status: 'Error' })],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    await expect(forwardRow).toContainText('Error')

    await clearSent(page)
    await forwardRow.getByTestId('port-forward-restart').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    const detail = dialog.getByTestId('port-forward-start-dialog')
    await expect(detail).toContainText('default/web-1')
    await expect(detail).toContainText('8080')
    await expect.poll(() => sentTypes(page)).toContain('CheckPortAvailability')

    await clearSent(page)
    const requestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('StartPortForward')
    const calls = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    const startCall = calls.find((c) => c.type === 'StartPortForward')
    expect(startCall).toEqual({
      type: 'StartPortForward',
      data: { namespace: 'default', pod: 'web-1', local_port: 8080, remote_port: 8080, container: 'web' },
    })
    await finish(page, requestId, true)
    await expect(dialog).toBeHidden()
  })

  test('Scenario 4: an out-of-band PortForwardTunnel event updates a row live, no refresh needed', async ({
    page,
  }) => {
    await openPortForwardsPanel(page)
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ status: 'Active' })],
        }),
      ]),
    )
    const forwardRow = page.getByTestId('port-forward-row')
    await expect(forwardRow).toContainText('Active')

    // Simulates the pod restarting under the forward: Active -> Error, with no user action and
    // no fresh PortForwardsUpdated list refresh.
    await emit(
      page,
      batch([
        stream(2, globalRoute('network/portforwards'), {
          type: 'PortForwardTunnel',
          data: { tunnel_id: 'pf-1', status: 'Error', local_endpoint: null },
        }),
      ]),
    )
    await expect(forwardRow).toContainText('Error')
    await expect(forwardRow.getByTestId('port-forward-restart')).toBeVisible()
  })

  test('Scenario 5: a fresh PortForwardsUpdated (e.g. after a context switch) fully replaces the rows', async ({
    page,
  }) => {
    await openPortForwardsPanel(page)
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({ id: 'pf-old', namespace: 'kind-a-ns', pod: 'old-pod' })],
        }),
      ]),
    )
    await expect(page.getByTestId('port-forward-row')).toHaveCount(1)
    await expect(page.getByTestId('port-forward-target')).toContainText('kind-a-ns/old-pod')

    // The backend already tore down the prior context's forwards and sends a fresh, complete
    // list for the new context — the UI's only obligation is to render exactly what arrives.
    await emit(
      page,
      batch([
        list(2, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [],
        }),
      ]),
    )
    await expect(page.getByTestId('port-forward-row')).toHaveCount(0)
    await expect(page.getByText('No active port forwards')).toBeVisible()
  })
})

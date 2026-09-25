/**
 * T124 parity gaps: StatefulSets, DaemonSets, ReplicaSets, ReplicationControllers,
 * PriorityClasses, RuntimeClasses, Leases and ValidatingWebhookConfigurations each open from the
 * navigator, render their list from a backend event and send the kind's `Select…` on row click;
 * the workload kinds also send `Restart…` from the row menu, behind a confirmation.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, list, nsRoute } from '../fixtures/builders'
import { LEASE_ROW, PRIORITY_ROW, RUNTIME_ROW, VALIDATING_ROW } from '../fixtures/batchD'
import type { ResourceListEvent } from '../../src/protocol/types'
import {
  clearSent,
  confirmDialog,
  emit,
  expectRowCount,
  navigateTo,
  open,
  row,
  sent,
} from './helpers'

const wl = (name: string) => [
  { name, namespace: 'parity-ns', desired: 3, ready: 2, updated: 3, available: 2 },
]

interface Case {
  label: string
  group: string
  section: string
  event: ResourceListEvent
  key: string
  select: { type: string; data: object }
  cluster?: boolean
  restart?: string
}

const WORKLOADS: Case[] = [
  {
    label: 'StatefulSets',
    group: 'Workloads',
    section: 'workloads/statefulsets',
    event: { type: 'StatefulSetsUpdated', data: wl('db') },
    key: 'parity-ns/db',
    select: { type: 'SelectStatefulSet', data: { namespace: 'parity-ns', name: 'db' } },
    restart: 'RestartStatefulSet',
  },
  {
    label: 'DaemonSets',
    group: 'Workloads',
    section: 'workloads/daemonsets',
    event: { type: 'DaemonSetsUpdated', data: wl('agent') },
    key: 'parity-ns/agent',
    select: { type: 'SelectDaemonSet', data: { namespace: 'parity-ns', name: 'agent' } },
    restart: 'RestartDaemonSet',
  },
  {
    label: 'ReplicaSets',
    group: 'Workloads',
    section: 'workloads/replicasets',
    event: { type: 'ReplicaSetsUpdated', data: wl('web-7d4') },
    key: 'parity-ns/web-7d4',
    select: { type: 'SelectReplicaSet', data: { namespace: 'parity-ns', name: 'web-7d4' } },
    restart: 'RestartReplicaSet',
  },
  {
    label: 'ReplicationControllers',
    group: 'Workloads',
    section: 'workloads/replicationcontrollers',
    event: { type: 'ReplicationControllersUpdated', data: wl('legacy') },
    key: 'parity-ns/legacy',
    select: {
      type: 'SelectReplicationController',
      data: { namespace: 'parity-ns', name: 'legacy' },
    },
    restart: 'RestartReplicationController',
  },
]

const CONFIG: Case[] = [
  {
    label: 'Priority Classes',
    group: 'Config',
    section: 'config/priorityclasses',
    event: { type: 'PriorityClassesUpdated', data: [PRIORITY_ROW] },
    key: PRIORITY_ROW.name,
    select: { type: 'SelectPriorityClass', data: { name: PRIORITY_ROW.name } },
    cluster: true,
  },
  {
    label: 'Runtime Classes',
    group: 'Config',
    section: 'config/runtimeclasses',
    event: { type: 'RuntimeClassesUpdated', data: [RUNTIME_ROW] },
    key: RUNTIME_ROW.name,
    select: { type: 'SelectRuntimeClass', data: { name: RUNTIME_ROW.name } },
    cluster: true,
  },
  {
    label: 'Leases',
    group: 'Config',
    section: 'config/leases',
    event: { type: 'LeasesUpdated', data: [LEASE_ROW] },
    key: `${LEASE_ROW.namespace}/${LEASE_ROW.name}`,
    select: {
      type: 'SelectLease',
      data: { namespace: LEASE_ROW.namespace, name: LEASE_ROW.name },
    },
  },
  {
    label: 'Validating Webhook Configurations',
    group: 'Config',
    section: 'config/validatingwebhooks',
    event: { type: 'ValidatingWebhookConfigurationsUpdated', data: [VALIDATING_ROW] },
    key: VALIDATING_ROW.name,
    select: {
      type: 'SelectValidatingWebhookConfiguration',
      data: { name: VALIDATING_ROW.name },
    },
    cluster: true,
  },
]

const route = (c: Case) =>
  c.cluster
    ? { section: c.section, scope: 'Cluster' as const, context: 'kind-a', namespace: null }
    : nsRoute('kind-a', c.section)

async function openList(page: Page, c: Case) {
  await open(page)
  await navigateTo(page, c.label, c.group)
  await emit(page, batch([list(1, route(c), c.event)]))
  await expectRowCount(page, 1, c.section)
}

test.describe('parity: list and inspector', () => {
  for (const c of [...WORKLOADS, ...CONFIG]) {
    test(`${c.label}: list renders and row click sends ${c.select.type}`, async ({ page }) => {
      await openList(page, c)
      await expect(row(page, c.key, c.section)).toBeVisible()
      await clearSent(page)
      await row(page, c.key, c.section).click()
      await expect
        .poll(async () => (await sent(page)).find((x) => x.type.startsWith('Select')))
        .toEqual(c.select)
    })
  }
})

test.describe('parity: row-menu Restart', () => {
  for (const c of WORKLOADS) {
    test(`${c.label}: row menu Restart confirms and sends ${c.restart}`, async ({ page }) => {
      await openList(page, c)
      await row(page, c.key, c.section).click({ button: 'right' })
      const restart = page.getByTestId('action-restart')
      await expect(restart).toBeVisible()
      await clearSent(page)
      await restart.click()
      await expect(confirmDialog(page)).toBeVisible()
      // Nothing is sent until the confirmation is accepted.
      expect((await sent(page)).filter((x) => x.type.startsWith('Restart'))).toEqual([])
      await confirmDialog(page).getByTestId('confirm-submit').click()
      const [ns, name] = c.key.split('/')
      await expect
        .poll(() => sent(page))
        .toEqual([{ type: c.restart, data: { namespace: ns, name } }])
    })
  }
})

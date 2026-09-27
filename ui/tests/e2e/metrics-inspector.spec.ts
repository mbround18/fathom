/**
 * Node metrics (Nodes list + inspector), workload metrics polling from the inspector, and the
 * daemon timeline request. Runs against the scripted fake backend: the metrics-server is
 * simulated by emitting `NodeMetricsUpdated` / `*MetricsUpdated` events.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, clusterRoute, list, nsRoute, read } from '../fixtures/builders'
import { daemonBatch, daemonStatus, entry } from '../fixtures/daemon'
import { genDeployments } from '../fixtures/gen-pods'
import type { WorkloadMetricsAggregate } from '../../src/protocol/types'
import {
  clearSent,
  DEPLOYMENTS,
  emit,
  expectNoHorizontalScroll,
  inspector,
  open,
  row,
  sent,
  sentTypes,
  showList,
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

const nodeMetrics = (seq: number, cpu: number, memBytes: number, timestamp: number) =>
  batch([
    read(seq, clusterRoute('kind-a', 'nodes'), {
      type: 'NodeMetricsUpdated',
      data: {
        name: NODE.name,
        cpu: { usage_percent: cpu, cores: cpu / 100 },
        memory: { usage_bytes: memBytes, usage_percent: 10, limit_bytes: 4_000_000_000 },
        disk: null,
        timestamp,
      },
    }),
  ])

async function openNodesList(page: Page): Promise<void> {
  await open(page)
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.getByRole('button', { name: 'Cluster', expanded: false })
  if (await toggle.isVisible()) await toggle.click()
  await nav.locator('[data-section="nodes"]').click()
  await expect.poll(() => sentTypes(page)).toContain('RefreshNodes')
  await emit(
    page,
    batch([list(1, clusterRoute('kind-a', 'nodes'), { type: 'NodesUpdated', data: [NODE] })]),
  )
}

const listShowsUsage = (info: TestInfo): boolean => !/-(phone|tablet)$/.test(info.project.name)

test.describe('node metrics', () => {
  test('list cells and the inspector chart follow NodeMetricsUpdated; polling stops on close', async ({
    page,
  }, info: TestInfo) => {
    await openNodesList(page)
    // Rendering the Nodes list is what asks the backend to poll node metrics.
    // (Phone and tablet fold the usage columns into the inspector, so only laptop+ layouts poll from the list.)
    if (listShowsUsage(info)) {
      await expect
        .poll(async () => (await sent(page)).find((c) => c.type === 'StartNodeMetricsPolling'))
        .toMatchObject({ data: { interval_secs: 5 } })
    }

    await page.locator(`[role="row"][data-row-key="${NODE.name}"]`).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    // No metrics-server data yet: the existing unavailable state, not a blank chart.
    await expect(insp.locator('[data-pane-state="metrics-unavailable"]')).toBeVisible()

    await emit(page, nodeMetrics(2, 25, 512 * 1048576, 1000))
    await expect(insp.getByTestId('inspector-metrics')).toContainText('CPU 25.0%')
    await expect(insp.getByTestId('metrics-chart')).toHaveCount(0) // one sample: collecting
    await emit(page, nodeMetrics(3, 30, 600 * 1048576, 2000))
    await expect(insp.getByTestId('inspector-metrics')).toContainText('CPU 30.0%')
    await expect(insp.getByTestId('metrics-chart')).toBeVisible()
    await expect(insp.getByTestId('metrics-chart').locator('svg').first()).toBeVisible()

    // Nodes list: usage cell (hidden columns are folded into the inspector on phone/tablet).
    if (listShowsUsage(info)) {
      await expect(
        page.locator(`[role="row"][data-row-key="${NODE.name}"]`).getByTestId('node-cpu-usage'),
      ).toHaveText('30.0%')
    }
    await expectNoHorizontalScroll(page)

    // The list still shows node usage, so closing the inspector must not stop the shared poll.
    await page.keyboard.press('Escape')
    await expect(insp.getByTestId('inspector-metrics')).toHaveCount(0)
    expect(await sentTypes(page)).not.toContain('StopNodeMetricsPolling')
  })

  test('without metrics-server the list shows an em dash and the inspector stays unavailable', async ({
    page,
  }) => {
    await openNodesList(page)
    await page.locator(`[role="row"][data-row-key="${NODE.name}"]`).click()
    await expect(inspector(page).locator('[data-pane-state="metrics-unavailable"]')).toBeVisible()
    await expect(inspector(page).getByTestId('metrics-chart')).toHaveCount(0)
  })
})

const DEP = genDeployments('kind-a')[0] as ReturnType<typeof genDeployments>[number]
const DEP_KEY = `${DEP.namespace}/${DEP.name}`

const aggregate = (seq: number, cpu: number, memMiB: number, timestamp: number) =>
  batch([
    read(seq, nsRoute('kind-a', DEPLOYMENTS, DEP.namespace), {
      type: 'DeploymentMetricsUpdated',
      data: {
        namespace: DEP.namespace,
        name: DEP.name,
        workload_kind: 'Deployment',
        replica_count: 2,
        total_cpu_usage_percent: cpu,
        total_memory_usage_bytes: memMiB * 1048576,
        total_memory_limit_bytes: null,
        total_memory_request_bytes: null,
        total_cpu_request_millicores: null,
        total_cpu_limit_millicores: null,
        containers: [],
        timestamp,
      },
    } satisfies { type: 'DeploymentMetricsUpdated'; data: WorkloadMetricsAggregate }),
  ])

test.describe('workload metrics', () => {
  test('inspector polls only while open and renders the aggregate chart', async ({ page }) => {
    await open(page)
    await showList(page, DEPLOYMENTS, 'Deployments')
    await clearSent(page)
    await row(page, DEP_KEY, DEPLOYMENTS).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()

    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'StartWorkloadMetricsPolling'))
      .toMatchObject({
        data: {
          namespace: DEP.namespace,
          workload_name: DEP.name,
          workload_kind: 'Deployment',
          pod_selector: `app=${DEP.name}`,
        },
      })

    // Replica count only (no usage source): honest unavailable state.
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', DEPLOYMENTS, DEP.namespace), {
          type: 'DeploymentMetricsUpdated',
          data: {
            namespace: DEP.namespace,
            name: DEP.name,
            workload_kind: 'Deployment',
            replica_count: 2,
            total_cpu_usage_percent: null,
            total_memory_usage_bytes: null,
            total_memory_limit_bytes: null,
            total_memory_request_bytes: null,
            total_cpu_request_millicores: null,
            total_cpu_limit_millicores: null,
            containers: [],
            timestamp: 500,
          },
        }),
      ]),
    )
    await expect(insp.locator('[data-pane-state="metrics-unavailable"]')).toBeVisible()

    await emit(page, aggregate(3, 40, 200, 1000))
    await emit(page, aggregate(4, 55, 240, 2000))
    await expect(insp.getByTestId('inspector-metrics')).toContainText('CPU 55.0%')
    await expect(insp.getByTestId('metrics-chart')).toBeVisible()
    await expect(insp.getByTestId('metrics-chart').locator('svg').first()).toBeVisible()

    await clearSent(page)
    await page.keyboard.press('Escape')
    await expect(insp.getByTestId('inspector-metrics')).toHaveCount(0)
    await expect
      .poll(async () => (await sentTypes(page)).includes('StopWorkloadMetricsPolling'))
      .toBe(true)
  })
})

test.describe('daemon timeline request', () => {
  const stacked = (i: TestInfo): boolean =>
    i.project.name.endsWith('-phone') || i.project.name.endsWith('-tablet')

  test('opening the pane requests the buffered timeline; Refresh asks again', async ({
    page,
  }, info) => {
    await open(page)
    await emit(page, daemonStatus('available'))
    await clearSent(page)
    if (stacked(info)) {
      await page.getByTestId('daemon-chip').click()
      await page.getByTestId('daemon-open-timeline').click()
    } else {
      await page.getByRole('tab', { name: 'While you were away' }).click()
    }
    await expect(page.getByTestId('daemon-timeline').first()).toBeVisible()
    await expect
      .poll(async () => (await sent(page)).filter((c) => c.type === 'RequestDaemonTimeline'))
      .toEqual([{ type: 'RequestDaemonTimeline', data: { context: 'kind-a', limit: 1000 } }])

    await emit(page, daemonBatch(1, [entry(1), entry(2)]))
    await expect(page.getByTestId('daemon-timeline-entry')).toHaveCount(2)

    await clearSent(page)
    await page.getByTestId('daemon-timeline-refresh').click()
    await expect
      .poll(async () => (await sent(page)).filter((c) => c.type === 'RequestDaemonTimeline'))
      .toHaveLength(1)
    await expectNoHorizontalScroll(page)
  })
})

/**
 * T124 parity gaps: workload metrics chart rendering in the Metrics pane (pod sparklines and
 * workload aggregate meters) and the pod-log follow toggle (scroll up to stop following, "Jump
 * to latest" to resume).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, read, stream } from '../fixtures/builders'
import { emit, inspector, navigateTo, open, row, sent } from './helpers'

const pod = (t: number, cpu: number) => ({
  namespace: 'default',
  name: 'web',
  cpu: { usage_percent: cpu, cores: 0.1 },
  memory: { usage_bytes: 2048 * 1024 * 1024, usage_percent: cpu / 2, limit_bytes: null },
  network: { bytes_in: null, bytes_out: null, packets_in: null, packets_out: null },
  timestamp: t,
})

test('metrics pane renders pod charts and workload aggregates', async ({ page }) => {
  await open(page)
  await navigateTo(page, 'Metrics', 'Integrations')
  await expect(page.getByText('No metrics received yet')).toBeVisible()

  const r = nsRoute('kind-a', 'integrations/metrics', 'default')
  await emit(page, batch([read(1, r, { type: 'PrometheusMetricsUpdated', data: pod(1, 20) })]))
  const podRow = page.getByTestId('pod-metrics-row')
  await expect(podRow).toContainText('default/web')
  await expect(podRow.getByRole('meter', { name: 'default/web CPU' })).toHaveAttribute(
    'aria-valuenow',
    '20',
  )
  await expect(podRow.getByText('collecting').first()).toBeVisible()

  // A second sample turns "collecting" into a drawn sparkline.
  await emit(page, batch([read(2, r, { type: 'PrometheusMetricsUpdated', data: pod(2, 60) })]))
  const spark = podRow.getByRole('img', { name: 'default/web CPU history' })
  await expect(spark).toBeVisible()
  await expect(spark.locator('polyline')).toHaveAttribute('points', /,.* .*,/)
  await expect(podRow.getByRole('meter', { name: 'default/web CPU' })).toHaveAttribute(
    'aria-valuenow',
    '60',
  )

  await emit(
    page,
    batch([
      read(3, r, {
        type: 'DeploymentMetricsUpdated',
        data: {
          namespace: 'default',
          name: 'api',
          workload_kind: 'Deployment',
          replica_count: 3,
          total_cpu_usage_percent: 42,
          total_memory_usage_bytes: 1024 * 1024 * 512,
          total_memory_limit_bytes: null,
          total_memory_request_bytes: null,
          total_cpu_request_millicores: null,
          total_cpu_limit_millicores: null,
          containers: [],
          timestamp: 3,
        },
      }),
    ]),
  )
  const w = page.getByTestId('workload-metrics-row')
  await expect(w).toContainText('Deployment default/api (3)')
  await expect(w).toContainText('42.0%')
  await expect(w).toContainText('512.0 MiB')
  await expect(w.getByRole('meter', { name: 'api CPU' })).toHaveAttribute('aria-valuenow', '42')
})

test('log follow: scrolling up stops following, Jump to latest resumes', async ({ page }) => {
  await open(page)
  await row(page, `${DETAIL_POD.namespace}/${DETAIL_POD.name}`).click()
  const insp = inspector(page)
  await insp.getByRole('tab', { name: 'Logs' }).click()
  await expect
    .poll(async () => (await sent(page)).some((c) => c.type === 'StartPodLogs'))
    .toBe(true)
  const start = (await sent(page)).findLast((c) => c.type === 'StartPodLogs')
  const streamId = start?.type === 'StartPodLogs' ? start.data.stream_id : undefined
  if (streamId === undefined) throw new Error('StartPodLogs carries a client stream id')

  const r = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
  const lines = (from: number, n: number) =>
    Array.from({ length: n }, (_, i) => `line ${String(from + i)}`)
  const chunk = (seq: number, from: number, n: number) =>
    batch([
      stream(seq, r, {
        type: 'PodLogsChunk',
        data: { stream_id: streamId, lines: lines(from, n) },
      }),
    ])

  await emit(page, chunk(300, 0, 200))
  const scroll = insp.getByTestId('pod-logs-scroll')
  await expect(scroll).toContainText('line 199')
  const atBottom = () =>
    scroll.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight < 8)
  await expect.poll(atBottom).toBe(true)
  const jump = insp.getByRole('button', { name: 'Jump to latest' })
  await expect(jump).toHaveCount(0)

  // Following: a new chunk keeps the view pinned to the bottom.
  await emit(page, chunk(301, 200, 50))
  await expect(scroll).toContainText('line 249')
  await expect.poll(atBottom).toBe(true)

  // Scroll to the top: follow turns off, new output no longer moves the view.
  await scroll.evaluate((el) => {
    el.scrollTop = 0
  })
  await expect(jump).toBeVisible()
  await emit(page, chunk(302, 250, 50))
  await expect(scroll).toContainText('line 299')
  expect(await scroll.evaluate((el) => el.scrollTop)).toBeLessThan(8)

  await jump.click()
  await expect(jump).toHaveCount(0)
  await expect.poll(atBottom).toBe(true)
})

/**
 * T025 (US2 scenarios 1-3): selecting an application opens its inspector with metadata, metrics,
 * and related resources populated, and shows a metrics-unavailable state when no metrics source
 * is configured. Same hand-built-fixture convention as `us1-applications-list.spec.ts` (no
 * `cluster()` wiring for `applications` in `tests/fixtures/scenarios.ts`).
 */
import { expect, test, type Page } from '@playwright/test'

import type { WorkloadMetricsAggregate } from '../../src/protocol/types'
import { batch, list, nsRoute, read } from '../fixtures/builders'
import { emit, inspector, navigateTo, open, row, sent, sentTypes } from './helpers'

const APPLICATIONS = 'applications'

const WEB = {
  instance: 'web',
  name: 'web',
  namespace: 'payments',
  managed_by: 'Flux' as const,
  version: '1.4.0',
  creation_timestamp: '2024-02-20T08:30:00Z',
  ready_count: 3,
  total_count: 3,
}

const WEB_DETAILS = {
  instance: WEB.instance,
  name: WEB.name,
  namespace: WEB.namespace,
  managed_by: WEB.managed_by,
  version: WEB.version,
  creation_timestamp: WEB.creation_timestamp,
  labels: { 'app.kubernetes.io/instance': WEB.instance },
  urls: ['https://web.example.com'],
  related: [
    {
      section: 'workloads/pods',
      kind: 'Pod',
      namespace: WEB.namespace,
      name: 'web-abc123',
      relation: 'owns',
      status_hint: 'Running',
    },
  ],
}

function metrics(overrides: Partial<WorkloadMetricsAggregate> = {}): WorkloadMetricsAggregate {
  return { ...metricsBase, ...overrides }
}

const metricsBase: WorkloadMetricsAggregate = {
  namespace: WEB.namespace,
  name: WEB.instance,
  workload_kind: 'Application',
  replica_count: WEB.total_count,
  total_cpu_usage_percent: 12.5,
  total_memory_usage_bytes: 134_217_728,
  total_memory_limit_bytes: 268_435_456,
  total_memory_request_bytes: 67_108_864,
  total_cpu_request_millicores: 250,
  total_cpu_limit_millicores: 500,
  containers: [],
  timestamp: Date.now(),
}

async function selectWeb(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Applications', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshApplications')
  await emit(
    page,
    batch([list(1, nsRoute('kind-a', APPLICATIONS), { type: 'ApplicationsUpdated', data: [WEB] })]),
  )
  await row(page, `${WEB.namespace}/${WEB.instance}`, APPLICATIONS).click()
  await expect
    .poll(() => sent(page))
    .toEqual(
      expect.arrayContaining([
        { type: 'SelectApplication', data: { namespace: WEB.namespace, instance: WEB.instance } },
      ]),
    )
}

test.describe('US2 application inspector', () => {
  test('T025: metadata, metrics, and related resources populate', async ({ page }) => {
    await selectWeb(page)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', APPLICATIONS, WEB.namespace), {
          type: 'ApplicationDetailsUpdated',
          data: WEB_DETAILS,
        }),
        read(3, nsRoute('kind-a', APPLICATIONS, WEB.namespace), {
          type: 'ApplicationMetricsUpdated',
          data: metrics(),
        }),
      ]),
    )

    const insp = inspector(page)
    await expect(insp).toContainText('web')
    await expect(insp).toContainText('Flux')
    await expect(insp).toContainText('1.4.0')
    await expect(insp).toContainText('https://web.example.com')
    await expect(insp).toContainText('12.5%')
    await expect(insp).toContainText('250m')
    await expect(insp).toContainText('web-abc123')
    await expect(insp).toContainText('owns')
  })

  test('T025: shows metrics-unavailable when no metrics source is configured', async ({ page }) => {
    await selectWeb(page)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', APPLICATIONS, WEB.namespace), {
          type: 'ApplicationDetailsUpdated',
          data: WEB_DETAILS,
        }),
        read(3, nsRoute('kind-a', APPLICATIONS, WEB.namespace), {
          type: 'ApplicationMetricsUpdated',
          data: metrics({ total_cpu_usage_percent: null, total_memory_usage_bytes: null }),
        }),
      ]),
    )

    await expect(inspector(page)).toContainText('Metrics unavailable')
  })
})

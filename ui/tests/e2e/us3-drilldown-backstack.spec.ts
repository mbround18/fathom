/**
 * T036 (US3 scenarios 1-4): Application -> Pod -> Node drill-down builds a breadcrumb, two
 * "back" clicks restore prior targets from cache without re-fetching, and a related link to a
 * since-deleted target shows "not found" once its section's list reply comes back without it
 * (T050's generic mechanism in `openInspector`/`Inspector.tsx`). Same hand-built-fixture
 * convention as the other Applications specs (no `cluster()` wiring for `applications`/`nodes`).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, list, nsRoute, read } from '../fixtures/builders'
import { clearSent, emit, inspector, navigateTo, open, row, sent, sentTypes } from './helpers'

const APPLICATIONS = 'applications'
const NODES = 'nodes'

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

const POD_NAME = 'web-abc123'
const NODE_NAME = 'kind-a-worker'

const WEB_DETAILS = {
  instance: WEB.instance,
  name: WEB.name,
  namespace: WEB.namespace,
  managed_by: WEB.managed_by,
  version: WEB.version,
  creation_timestamp: WEB.creation_timestamp,
  labels: {},
  urls: [],
  related: [
    {
      section: 'workloads/pods',
      kind: 'Pod',
      namespace: WEB.namespace,
      name: POD_NAME,
      relation: 'owns',
      status_hint: 'Running',
    },
  ],
}

const POD_ROW = {
  name: POD_NAME,
  namespace: WEB.namespace,
  phase: 'Running',
  node: NODE_NAME,
  ready: 1,
  total: 1,
  restarts: 0,
}

const NODE_ROW = {
  name: NODE_NAME,
  status: 'Ready',
  role: 'worker',
  pod_count: 8,
  pod_capacity: 110,
  cpu_allocatable_millicores: 2000,
  memory_allocatable_bytes: 4_000_000_000,
}

async function openWebApplication(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Applications', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshApplications')
  await emit(
    page,
    batch([list(1, nsRoute('kind-a', APPLICATIONS), { type: 'ApplicationsUpdated', data: [WEB] })]),
  )
  await row(page, `${WEB.namespace}/${WEB.instance}`, APPLICATIONS).click()
  await expect.poll(() => sentTypes(page)).toContain('SelectApplication')
  await emit(
    page,
    batch([
      read(2, nsRoute('kind-a', APPLICATIONS, WEB.namespace), {
        type: 'ApplicationDetailsUpdated',
        data: WEB_DETAILS,
      }),
    ]),
  )
}

test.describe('US3 drill-down back-stack', () => {
  test('T036: Application -> Pod -> Node builds a breadcrumb; two backs restore from cache', async ({
    page,
  }) => {
    await openWebApplication(page)
    const insp = inspector(page)
    await clearSent(page)

    // Drill into the related Pod.
    await insp.getByRole('button').filter({ hasText: POD_NAME }).click()
    await expect
      .poll(() => sentTypes(page))
      .toEqual(expect.arrayContaining(['SelectPod', 'RefreshRelatedResources', 'RefreshPods']))
    expect(await sent(page)).toEqual(
      expect.arrayContaining([
        { type: 'SelectPod', data: { namespace: WEB.namespace, name: POD_NAME } },
      ]),
    )
    await emit(
      page,
      batch([
        list(3, nsRoute('kind-a', 'workloads/pods'), { type: 'PodsUpdated', data: [POD_ROW] }),
        read(4, nsRoute('kind-a', 'workloads/pods', WEB.namespace), {
          type: 'PodDetailsUpdated',
          data: {
            namespace: POD_ROW.namespace,
            name: POD_ROW.name,
            creation_timestamp: '2024-03-01T00:00:00Z',
            labels: {},
            annotations: {},
            containers: [],
            ports: [],
            resource_specs: [],
            owner: null,
            resolved_owner: null,
            persistent_volume_claims: [],
            raw_json: '{}',
          },
        }),
        read(5, nsRoute('kind-a', 'workloads/pods', WEB.namespace), {
          type: 'RelatedResourcesUpdated',
          data: {
            section: 'workloads/pods',
            kind: 'Pod',
            namespace: WEB.namespace,
            name: POD_NAME,
            related: [
              { section: NODES, kind: 'Node', namespace: null, name: NODE_NAME, relation: 'scheduled-on', status_hint: 'Ready' },
            ],
          },
        }),
      ]),
    )

    const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb' })
    await expect(breadcrumb).toBeVisible()
    await expect(breadcrumb).toContainText('web')
    await expect(breadcrumb).toContainText(POD_NAME)
    await expect(insp).not.toContainText('No longer exists')

    // Drill into the related Node.
    await clearSent(page)
    await insp.getByRole('button').filter({ hasText: NODE_NAME }).click()
    await expect
      .poll(() => sentTypes(page))
      .toEqual(expect.arrayContaining(['SelectNode', 'RefreshRelatedResources', 'RefreshNodes']))
    await emit(
      page,
      batch([list(6, clusterRoute('kind-a', NODES), { type: 'NodesUpdated', data: [NODE_ROW] })]),
    )
    await expect(breadcrumb).toContainText('web')
    await expect(breadcrumb).toContainText(POD_NAME)
    await expect(breadcrumb).toContainText(NODE_NAME)
    await expect(insp).not.toContainText('No longer exists')

    // Back once: restores the Pod, no re-fetch.
    await clearSent(page)
    await breadcrumb.getByRole('button', { name: '← Back' }).click()
    await expect(breadcrumb).toContainText('web')
    await expect(breadcrumb).toContainText(POD_NAME)
    await expect(page.getByRole('navigation', { name: 'Breadcrumb' })).not.toContainText(NODE_NAME)
    expect(await sentTypes(page)).toEqual([])

    // Back again: restores the Application; breadcrumb disappears (root, no history left).
    await page.getByRole('navigation', { name: 'Breadcrumb' }).getByRole('button', { name: '← Back' }).click()
    await expect(page.getByRole('navigation', { name: 'Breadcrumb' })).toHaveCount(0)
    await expect(insp).toContainText('web')
    expect(await sentTypes(page)).toEqual([])
  })

  test('T036: a related link to a since-deleted target shows "not found" within its list reply', async ({
    page,
  }) => {
    await openWebApplication(page)
    const insp = inspector(page)
    await clearSent(page)

    await insp.getByRole('button').filter({ hasText: POD_NAME }).click()
    await expect.poll(() => sentTypes(page)).toContain('RefreshPods')
    // The pod was deleted out of band: its list reply no longer contains it.
    await emit(
      page,
      batch([list(3, nsRoute('kind-a', 'workloads/pods'), { type: 'PodsUpdated', data: [] })]),
    )

    await expect(insp).toContainText('No longer exists')
  })
})

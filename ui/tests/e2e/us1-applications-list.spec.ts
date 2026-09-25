/**
 * T013 (US1 scenario 1): the Applications section lists Helm- and Flux-managed applications,
 * with the expected columns, and respects the active namespace filter — Applications has no
 * `cluster()` wiring in `tests/fixtures/scenarios.ts` (same position as Nodes), so this follows
 * `us2-node-actions.spec.ts`'s precedent of hand-built fixtures emitted directly in the spec.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, list, nsRoute } from '../fixtures/builders'
import {
  chooseScope,
  emit,
  expectRowCount,
  navigateTo,
  open,
  row,
  sent,
  sentTypes,
  table,
} from './helpers'

const APPLICATIONS = 'applications'

const REDIS = {
  instance: 'redis',
  name: 'redis',
  namespace: 'default',
  managed_by: 'Helm' as const,
  version: '7.2.4',
  creation_timestamp: '2024-01-15T10:00:00Z',
  ready_count: 1,
  total_count: 1,
}

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

async function goToApplications(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Applications', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshApplications')
}

test.describe('US1 applications list', () => {
  test('T013: lists Helm- and Flux-managed applications with the expected columns', async ({
    page,
  }, info) => {
    // Column visibility is responsive to pane width (`layout/column-budget.ts`), and every
    // project except desktop squeezes the single list pane too narrow to show columns below
    // priority 1-2; the full column set is only asserted on the desktop project, matching
    // `us1-pods-list.spec.ts`'s viewport-conditional convention (`isPhone`).
    const fullColumns = info.project.name === 'chromium-desktop'

    await goToApplications(page)
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', APPLICATIONS), {
          type: 'ApplicationsUpdated',
          data: [REDIS, WEB],
        }),
      ]),
    )
    await expectRowCount(page, 2, APPLICATIONS)

    const redisRow = row(page, `${REDIS.namespace}/${REDIS.instance}`, APPLICATIONS)
    await expect(redisRow).toContainText('redis')
    const webRow = row(page, `${WEB.namespace}/${WEB.instance}`, APPLICATIONS)
    await expect(webRow).toContainText('web')

    if (fullColumns) {
      await expect(redisRow).toContainText('default')
      await expect(redisRow).toContainText('Helm')
      await expect(redisRow).toContainText('7.2.4')
      await expect(webRow).toContainText('payments')
      await expect(webRow).toContainText('Flux')
    }
  })

  test('T013: respects the active namespace filter', async ({ page }) => {
    await goToApplications(page)
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', APPLICATIONS), {
          type: 'ApplicationsUpdated',
          data: [REDIS, WEB],
        }),
      ]),
    )
    await expectRowCount(page, 2, APPLICATIONS)

    await chooseScope(page, 'Namespace', 'payments')
    expect((await sent(page)).filter((c) => c.type === 'SetNamespaceFilter').at(-1)).toEqual({
      type: 'SetNamespaceFilter',
      data: { namespaces: ['payments'] },
    })

    await emit(
      page,
      batch([
        list(2, nsRoute('kind-a', APPLICATIONS), { type: 'ApplicationsUpdated', data: [WEB] }),
      ]),
    )
    await expectRowCount(page, 1, APPLICATIONS)
    await expect(table(page, APPLICATIONS)).toContainText('web')
    await expect(table(page, APPLICATIONS)).not.toContainText('redis')
  })
})

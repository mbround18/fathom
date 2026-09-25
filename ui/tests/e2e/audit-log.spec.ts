/**
 * 026-mutation-audit-log-viewer: one end-to-end scenario (a completed mutation appears in the
 * viewer with target, outcome and timestamp) plus one filter check (narrow, then clear).
 */
import { expect, test } from '@playwright/test'

import type { AuditLogEntry } from '../../src/protocol/types'
import { batch, system } from '../fixtures/builders'
import { emit, navigateTo, open, sentTypes } from './helpers'

const CORDON: AuditLogEntry = {
  id: 'a1',
  timestamp: '2026-03-01T10:00:00+00:00',
  action: 'nodeCordon',
  actionLabel: 'Node Cordon',
  context: 'kind-a',
  targetKind: 'Node',
  targetNamespace: null,
  targetName: 'kind-a-worker',
  outcome: { type: 'Success' },
  requestId: '7',
}
const DELETE: AuditLogEntry = {
  ...CORDON,
  id: 'a2',
  timestamp: '2026-03-01T11:00:00+00:00',
  action: 'delete',
  actionLabel: 'Delete',
  targetKind: 'Pod',
  targetNamespace: 'default',
  targetName: 'web-1',
  outcome: { type: 'Failure', data: { reason: 'forbidden' } },
}

test('a completed mutation appears in the audit log, and filters narrow then clear', async ({
  page,
}) => {
  await open(page)
  await navigateTo(page, 'Audit Log', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('RefreshAuditLog')
  await expect(page.getByText('No actions recorded yet')).toBeVisible()

  // Backend sends oldest first.
  await emit(
    page,
    batch([system(1, { type: 'AuditLogLoaded', data: { entries: [CORDON, DELETE] } })]),
  )
  const entries = page.getByTestId('audit-entry')
  await expect(entries).toHaveCount(2)
  await expect(entries.first()).toContainText('Delete')
  await expect(entries.nth(1)).toContainText('kind-a-worker')
  await expect(entries.nth(1).getByTestId('audit-outcome')).toHaveText('Success')
  await expect(entries.first().getByTestId('audit-outcome')).toHaveText('Failed')
  await expect(entries.nth(1).locator('time')).toBeVisible()

  // Filter check.
  await page.getByLabel('Search target name').fill('web')
  await expect(entries).toHaveCount(1)
  await expect(entries.first()).toContainText('web-1')
  await page.getByLabel('Search target name').fill('zzz')
  await expect(page.getByText('No matching entries')).toBeVisible()
  await page.getByRole('button', { name: 'Clear filters' }).click()
  await expect(entries).toHaveCount(2)
})

test('the viewer states it is local-only and never fetches cluster audit data', async ({
  page,
}) => {
  await open(page)
  await navigateTo(page, 'Audit Log', 'Team')
  await expect(page.getByTestId('audit-log-note')).toContainText('not Kubernetes')
  await expect.poll(() => sentTypes(page)).toContain('RefreshAuditLog')
  expect((await sentTypes(page)).some((t) => /cluster.?audit/i.test(t))).toBe(false)
})

/**
 * Batch A: Jobs and CronJobs lists render from backend events; Job delete goes through the
 * confirm dialog and sends exactly DeleteJob.
 */
import { expect, test } from '@playwright/test'

import { batch, list, nsRoute } from '../fixtures/builders'
import { CRONJOBS, JOBS } from '../fixtures/batchA'
import { check, clearSent, confirmDialog, emit, open, row, sent, showList } from './helpers'

const JOBS_SECTION = 'workloads/jobs'
const CRON_SECTION = 'workloads/cronjobs'

test('jobs: delete is confirmed and sends DeleteJob', async ({ page }) => {
  await open(page)
  await showList(page, JOBS_SECTION, 'Jobs')
  await emit(
    page,
    batch([list(50, nsRoute('kind-a', JOBS_SECTION), { type: 'JobsUpdated', data: JOBS })]),
  )
  await check(page, 'batch-ns/flaky-1', JOBS_SECTION)
  await clearSent(page)
  await page.getByTestId('bulk-delete').click()
  await expect(confirmDialog(page).getByTestId('confirm-item')).toHaveCount(1)
  await confirmDialog(page).getByTestId('confirm-submit').click()
  await expect
    .poll(() => sent(page))
    .toEqual([{ type: 'DeleteJob', data: { namespace: 'batch-ns', name: 'flaky-1' } }])
})

test('cronjobs: suspended state is shown', async ({ page }) => {
  await open(page)
  await showList(page, CRON_SECTION, 'CronJobs')
  await emit(
    page,
    batch([list(51, nsRoute('kind-a', CRON_SECTION), { type: 'CronJobsUpdated', data: CRONJOBS })]),
  )
  await expect(row(page, 'batch-ns/paused', CRON_SECTION)).toContainText('Suspended')
})

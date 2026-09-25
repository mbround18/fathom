/** 022-velero-backup-pane: completed/failed Backup rendering and the not-installed empty state. */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { VELERO_HEALTHY_AND_FAILED, VELERO_NOT_INSTALLED } from '../fixtures/velero'
import { emit, open } from './helpers'

async function openVeleroTab(page: Page, info: TestInfo) {
  test.skip(!info.project.name.endsWith('-desktop'), 'Velero lives in the desktop dock')
  await open(page)
  await page.getByRole('tab', { name: 'Velero' }).click()
}

test.describe('Velero pane', () => {
  test('renders completed and failed backups with tone and age', async ({ page }, info) => {
    await openVeleroTab(page, info)
    await emit(
      page,
      batch([system(1, { type: 'VeleroOverviewUpdated', data: VELERO_HEALTHY_AND_FAILED })]),
    )
    const rows = page.getByTestId('velero-backup-row')
    await expect(rows).toHaveCount(2)
    const ok = rows.filter({ hasText: 'nightly-ok' })
    const bad = rows.filter({ hasText: 'adhoc-bad' })
    await expect(ok).toHaveAttribute('data-tone', 'ok')
    await expect(ok).toContainText('Completed')
    await expect(ok).toContainText('2h ago')
    await expect(bad).toHaveAttribute('data-tone', 'err')
    await expect(bad).toContainText('Failed')
    await expect(bad).toContainText('5h ago')
  })

  test('shows an honest not-installed state', async ({ page }, info) => {
    await openVeleroTab(page, info)
    await emit(
      page,
      batch([system(1, { type: 'VeleroOverviewUpdated', data: VELERO_NOT_INSTALLED })]),
    )
    await expect(page.getByText('Velero is not installed', { exact: false }).first()).toBeVisible()
    await expect(page.getByTestId('velero-backup-row')).toHaveCount(0)
    await expect(page.getByRole('alert')).toHaveCount(0)
  })
})

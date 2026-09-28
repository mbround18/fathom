/** 022-velero-backup-pane: completed/failed Backup rendering and the not-installed empty state. */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { VELERO_HEALTHY_AND_FAILED, VELERO_NOT_INSTALLED } from '../fixtures/velero'
import { emit, navigateTo, open } from './helpers'

/**
 * 043: Velero is listed only once detected (FR-036), and its home is Integrations › Velero.
 * Mark it installed first so the entry exists, then open it from the navigator.
 */
async function openVelero(page: Page): Promise<void> {
  await open(page)
  await emit(
    page,
    batch([system(1, { type: 'VeleroOverviewUpdated', data: VELERO_HEALTHY_AND_FAILED })]),
  )
  await navigateTo(page, 'Velero', 'Integrations')
}

test.describe('Velero pane', () => {
  test('renders completed and failed backups with tone and age', async ({ page }) => {
    await openVelero(page)
    await emit(
      page,
      batch([system(2, { type: 'VeleroOverviewUpdated', data: VELERO_HEALTHY_AND_FAILED })]),
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

  test('shows an honest not-installed state', async ({ page }) => {
    await openVelero(page)
    await emit(
      page,
      batch([system(2, { type: 'VeleroOverviewUpdated', data: VELERO_NOT_INSTALLED })]),
    )
    await expect(page.getByText('Velero is not installed', { exact: false }).first()).toBeVisible()
    await expect(page.getByTestId('velero-backup-row')).toHaveCount(0)
    await expect(page.getByRole('alert')).toHaveCount(0)
  })
})

/**
 * Spec 064 US4 (T025): approvers see what waits for them. The Runs nav item carries a neutral count
 * of the runs they could answer, which follows their answers; "Waiting for approval" lists the held
 * runs oldest first and says so plainly when there are none. People who can't approve see who can.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { hold, runInfo } from '../../support/fake-runs'

import { decision, freezeClock, openRun, signIn } from './support'

test.beforeEach(async ({ page }) => {
  await freezeClock(page)
})

const chip = (page: Page) => page.locator('[data-nav-badge="runs"]')
const rows = (page: Page) => page.getByRole('table', { name: 'Runs' }).locator('tbody tr')

async function approve(page: Page, id: string) {
  const panel = await openRun(page, id)
  await panel.locator('[data-approve]').click()
  const dialog = decision(page, 'approve')
  await expect(dialog.locator('[data-preview-summary]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toBeHidden()
}

test('the nav chip counts what waits for the approver, and follows their answers', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(hold({ id: 'run-0001', created_at: '2026-10-27T09:00:00Z' }))
  runs.add(hold({ id: 'run-0002', created_at: '2026-10-27T10:00:00Z' }))

  await page.goto('/runs')
  await expect(chip(page)).toHaveText('2')
  await expect(page.getByRole('link', { name: /Runs/ })).toContainText('2 runs waiting for your approval')

  await approve(page, 'run-0001')
  await expect(chip(page)).toHaveText('1')
  await expect(page.getByRole('link', { name: /Runs/ })).toContainText('1 run waiting for your approval')

  await approve(page, 'run-0002')
  await expect(chip(page)).toHaveCount(0)
})

test('the chip is neutral, never the accent of the current item', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(hold({ id: 'run-0001' }))
  await page.goto('/runs')
  const link = page.getByRole('navigation', { name: 'Sections' }).getByRole('link', { name: /Runs/ })
  await expect(link).toHaveAttribute('aria-current', 'page')
  await expect(chip(page)).toHaveText('1')
  const color = (sel: string) => page.locator(sel).evaluate((el) => getComputedStyle(el).color)
  expect(await color('[data-nav-badge="runs"]')).not.toBe(await color('nav a[aria-current="page"]'))
})

test('"Waiting for approval" lists the held runs, oldest first, and is kept in the address', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(hold({ id: 'run-0001', job_name: 'older', created_at: '2026-10-27T08:00:00Z' }, { needed: 2 }))
  runs.add(runInfo({ id: 'run-0002', job_name: 'done', state: 'succeeded', created_at: '2026-10-27T09:00:00Z' }))
  runs.add(hold({ id: 'run-0003', job_name: 'newer', created_at: '2026-10-27T10:00:00Z' }))

  await page.goto('/runs')
  const filter = page.locator('[data-runs-filter]')
  await expect(filter).toHaveAttribute('data-runs-filter', 'all')
  await expect(rows(page)).toHaveCount(3)

  await filter.getByRole('button', { name: 'Waiting for approval' }).click()
  await expect(page).toHaveURL(/\/runs\?state=pending_approval$/)
  await expect(filter).toHaveAttribute('data-runs-filter', 'pending_approval')
  await expect(filter.getByRole('button', { name: 'Waiting for approval' })).toHaveAttribute('aria-pressed', 'true')
  await expect(rows(page)).toHaveCount(2)
  await expect(rows(page).nth(0)).toContainText('older')
  await expect(rows(page).nth(0)).toContainText('Waiting · 0 of 2')
  await expect(rows(page).nth(1)).toContainText('newer')
  const listed = backend.received.flatMap((c) => (c.type === 'ListRuns' ? [c.data.state] : []))
  expect(listed).toContain('pending_approval')

  // Opened from a link, the filter is already on.
  await page.goto('/runs?state=pending_approval')
  await expect(filter).toHaveAttribute('data-runs-filter', 'pending_approval')
  await expect(rows(page)).toHaveCount(2)

  await filter.getByRole('button', { name: 'All' }).click()
  await expect(page).toHaveURL(/\/runs$/)
  await expect(rows(page)).toHaveCount(3)
})

test('with nothing waiting, the filter says so and clears', async ({ page, backend }) => {
  const runs = signIn(backend, 'approver')
  runs.add(runInfo({ id: 'run-0001', state: 'succeeded' }))
  await page.goto('/runs?state=pending_approval')
  const empty = page.locator('[data-empty="no-match"]')
  await expect(empty).toContainText('No runs waiting for approval.')
  await empty.getByRole('button', { name: 'Clear filter' }).click()
  await expect(page).toHaveURL(/\/runs$/)
  await expect(rows(page)).toHaveCount(1)
})

for (const role of ['viewer', 'operator'] as const) {
  test(`a${role === 'operator' ? 'n' : ''} ${role} sees who can approve, with no buttons and no chip`, async ({ page, backend }) => {
    const runs = signIn(backend, role)
    runs.add(hold({ id: 'run-0001' }))
    const panel = await openRun(page, 'run-0001')
    await expect(panel).toHaveAttribute('data-approval-panel', 'restricted')
    await expect(panel.locator('[data-approval-note]')).toHaveText('An approver or workspace admin can approve this.')
    await expect(panel.locator('[data-approve]')).toHaveCount(0)
    await expect(panel.locator('[data-deny]')).toHaveCount(0)
    await expect(chip(page)).toHaveCount(0)
    expect(backend.received.some((c) => c.type === 'CountPendingApprovals')).toBe(false)
  })
}

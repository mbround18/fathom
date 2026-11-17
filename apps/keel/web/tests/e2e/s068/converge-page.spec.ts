/**
 * Spec 068 T029 (contracts/ui.md "Converge page", "Runs list", "Hosts"): a converge run's page, its
 * matrix updated live, each cell opening the step's child run for that host, Cancel, and Retry
 * failed re-planned at the latest commit; the runs list showing the parent once; the Hosts
 * Converge column. At 360 px and 1280 px, against the fake backend.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { convergePlan, hostPlan } from '../../support/fake-converge'
import { converging, press, WIDTHS } from './setup'

const PLAN = convergePlan({
  hosts: [
    hostPlan('host-alpha', 'alpha', [{ job: 'debian' }, { job: 'keel-node' }]),
    hostPlan('host-bravo', 'bravo', [{ job: 'debian' }, { job: 'keel-node' }]),
  ],
})

/** A host's row (desktop) or card (phone), whichever shows. */
function host(page: Page, name: string) {
  return page.locator(`[data-converge-host="${name}"]:visible`)
}

function cell(page: Page, name: string, job: string) {
  return host(page, name).locator(`[data-step="${job}"]`)
}

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('the matrix follows the converge live and each cell opens its child run', async ({ page, backend }) => {
      const { converge } = converging(backend)
      const id = converge.begin(PLAN)
      await page.goto(`/runs/${id}`)
      await expect(page.getByRole('heading', { level: 1, name: 'Converge' })).toBeVisible()
      await expect(cell(page, 'alpha', 'debian')).toHaveAttribute('data-step-state', 'queued')
      await expect(cell(page, 'alpha', 'keel-node')).toHaveAttribute('data-step-state', 'blocked')

      converge.step(id, 'alpha', 1, 'running')
      await expect(cell(page, 'alpha', 'debian')).toHaveAttribute('data-step-state', 'running')
      await expect(cell(page, 'alpha', 'debian')).toHaveAccessibleName('debian on alpha: Running')
      await expect(host(page, 'alpha')).toHaveAttribute('data-host-state', 'holding')

      converge.step(id, 'alpha', 1, 'succeeded')
      await expect(cell(page, 'alpha', 'debian')).toHaveAttribute('data-step-state', 'succeeded')
      await expect(cell(page, 'alpha', 'debian')).toHaveAttribute('href', `/runs/${id}-alpha-1?host=alpha`)
      await cell(page, 'alpha', 'debian').click()
      await expect(page).toHaveURL(new RegExp(`/runs/${id}-alpha-1\\?host=alpha`))
    })

    test('Cancel converge stops the steps not yet sent', async ({ page, backend }) => {
      const { converge } = converging(backend)
      const id = converge.begin(PLAN)
      converge.step(id, 'alpha', 1, 'running')
      await page.goto(`/runs/${id}`)
      await expect(cell(page, 'alpha', 'debian')).toHaveAttribute('data-step-state', 'running')
      await press(page, page.getByRole('button', { name: 'Cancel converge' }))
      expect((await backend.waitForCmd('CancelConverge' as never)) as unknown).toMatchObject({ data: { run_id: id } })
      await expect(cell(page, 'alpha', 'keel-node')).toHaveAttribute('data-step-state', 'cancelled')
      await expect(cell(page, 'bravo', 'debian')).toHaveAttribute('data-step-state', 'cancelled')
      // The running step finishes on its own.
      await expect(cell(page, 'alpha', 'debian')).toHaveAttribute('data-step-state', 'running')
    })

    test('Retry failed previews the steps that did not succeed, re-planned at the latest commit', async ({ page, backend }) => {
      const { converge } = converging(backend)
      const id = converge.begin(PLAN)
      converge.step(id, 'alpha', 1, 'succeeded')
      converge.step(id, 'alpha', 2, 'succeeded')
      converge.step(id, 'bravo', 1, 'failed')
      converge.step(id, 'bravo', 2, 'skipped')
      await page.goto(`/runs/${id}`)
      await expect(cell(page, 'bravo', 'keel-node')).toHaveAttribute('data-step-state', 'skipped')
      await expect(page.getByRole('button', { name: 'Cancel converge' })).toHaveCount(0)

      await press(page, page.getByRole('checkbox', { name: 'Re-plan at latest commit' }), 'Space')
      await press(page, page.getByRole('button', { name: 'Retry failed' }))
      expect((await backend.waitForCmd('PreviewConvergeRetry' as never)) as unknown).toMatchObject({
        data: { run_id: id, replan: true },
      })
      const dialog = page.getByRole('dialog', { name: 'Retry failed steps' })
      await expect(dialog.locator('[data-plan-host="bravo"]:visible')).toBeVisible()
      await expect(dialog.locator('[data-plan-host="alpha"]:visible')).toHaveCount(0)
      const confirm = dialog.locator('[data-converge-confirm]')
      await expect(confirm).toHaveText('Converge 1 host (2 steps)')
      await press(page, confirm)
      await expect(dialog).toBeHidden()
      await expect(page.locator('[data-retry-of]')).toContainText(`converge ${id.slice(0, 7)}`)
    })

    test('the runs list shows a converge once, its steps on request', async ({ page, backend }) => {
      const { converge } = converging(backend)
      const id = converge.begin(PLAN)
      await page.goto('/runs')
      const table = page.locator('[data-run-kind="converge"]')
      await expect(table).toHaveCount(1)
      await expect(table.getByRole('link', { name: 'converge' })).toHaveAttribute('href', `/runs/${id}`)
      await press(page, page.getByRole('checkbox', { name: 'Show converge steps' }), 'Space')
      await expect(page).toHaveURL(/steps=1/)
      await expect(page.locator('[data-run-kind="converge"]')).toHaveCount(5)
    })

    test('Hosts shows where each host is in its converge', async ({ page, backend }) => {
      const { hosts, ops } = converging(backend)
      const ws = ops.id
      hosts.hosts = hosts.hosts.map((h) =>
        h.inventory_name === 'alpha'
          ? { ...h, converge: { run_id: 'conv-9', state: 'holding', step: 2, of: 3 } }
          : h.inventory_name === 'bravo'
            ? { ...h, converge: { run_id: 'conv-9', state: 'succeeded', step: 3, of: 3 } }
            : { ...h, workspace: ws },
      )
      await page.goto('/hosts')
      await expect(page.locator('[data-host-converge="holding"]')).toHaveText('converging 2/3')
      await expect(page.locator('[data-host-converge="succeeded"]')).toHaveText('converged')
      await expect(page.locator('[data-host-converge="succeeded"]')).toHaveAttribute('href', '/runs/conv-9')
    })
  })
}

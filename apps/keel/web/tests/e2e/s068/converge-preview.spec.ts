/**
 * Spec 068 T029 (contracts/ui.md "Hosts", "Host page", "Converge preview"): converging a selection
 * of hosts from Hosts, its preview (matrix or cards, params and credential names, policy, refusals,
 * check mode, signed-mode review) and Confirm by keyboard, which opens the converge page. At 360 px
 * and 1280 px, against the fake backend.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test } from '../../support/fake-backend'
import { convergePlan, hostPlan } from '../../support/fake-converge'
import { converging, press, WIDTHS } from './setup'

function preview(page: Page) {
  return page.getByRole('dialog', { name: 'Converge' })
}

/** The plan's host row (desktop) or card (phone), whichever shows. */
function planHost(page: Page, host: string) {
  return preview(page).locator(`[data-plan-host="${host}"]:visible`)
}

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('Converge… on a selection previews the plan and Confirm by keyboard opens the converge', async ({ page, backend }) => {
      const { converge } = converging(backend)
      converge.steps = [
        { job: 'debian', via: ['keel-node', 'debian'], params: { timezone: 'UTC' } },
        { job: 'keel-node', via: ['keel-node'], credential_names: ['registry_token'] },
      ]
      await page.goto('/hosts')
      await press(page, page.getByRole('checkbox', { name: 'Select alpha' }), 'Space')
      await press(page, page.getByRole('checkbox', { name: 'Select bravo' }), 'Space')
      await press(page, page.locator('[data-converge-selected]'))

      expect((await backend.waitForCmd('PreviewConverge' as never)) as unknown).toMatchObject({
        data: { binding_id: 'binding-ops', hosts: { kind: 'hosts', ids: ['host-alpha', 'host-bravo'] }, check: false },
      })
      const dialog = preview(page)
      await expect(dialog).toHaveAttribute('data-converge-preview', 'plan')
      await expect(dialog).toContainText('No approval needed')
      await expect(planHost(page, 'alpha')).toContainText('debian')
      await expect(planHost(page, 'bravo')).toContainText('keel-node')
      await expect(planHost(page, 'charlie')).toHaveCount(0)
      // Params and credential names, never values.
      await expect(dialog.getByRole('region', { name: 'Steps' })).toContainText('timezone')
      await expect(dialog.locator('[data-job="keel-node"] [data-credentials]')).toHaveText('registry_token')
      expect(converge.previewed).toHaveLength(1)

      const confirm = dialog.locator('[data-converge-confirm]')
      await expect(confirm).toHaveText('Converge 2 hosts (4 steps)')
      await press(page, confirm)
      await expect(dialog).toBeHidden()
      await expect(page).toHaveURL(/\/runs\/conv-2(\?|$)/)
      await expect(page.getByRole('heading', { level: 1, name: 'Converge' })).toBeVisible()
    })

    test('a host refused a step is left out of Confirm, with the reason', async ({ page, backend }) => {
      const { converge } = converging(backend)
      converge.planFor = (t) =>
        convergePlan({
          binding_id: t.binding_id,
          hosts: [
            hostPlan('host-alpha', 'alpha', [{ job: 'debian' }, { job: 'keel-node' }]),
            hostPlan('host-bravo', 'bravo', [{ job: 'debian' }], { refusals: [{ job: 'k3s-agent', reason: 'outside_ceiling' }] }),
          ],
          policy: { requires_approval: true, approvals_needed: 2, four_eyes: true, approval_ttl_secs: 14_400 },
        })
      await page.goto('/hosts')
      await page.getByRole('checkbox', { name: 'Select alpha' }).check()
      await page.getByRole('checkbox', { name: 'Select bravo' }).check()
      await page.locator('[data-converge-selected]').click()

      const dialog = preview(page)
      await expect(dialog).toContainText('Needs 2 approvals, four eyes, expires in 4 h (strictest of 2 steps)')
      await expect(planHost(page, 'bravo')).toHaveAttribute('data-included', 'false')
      await expect(planHost(page, 'bravo')).toContainText('k3s-agent: outside the host’s ceiling')
      await expect(dialog.locator('[data-converge-confirm]')).toHaveText('Converge 1 host (2 steps)')
    })

    test('Check only asks for the plan again in check mode', async ({ page, backend }) => {
      converging(backend)
      await page.goto('/hosts')
      await page.getByRole('checkbox', { name: 'Select alpha' }).check()
      await page.locator('[data-converge-selected]').click()
      const dialog = preview(page)
      await expect(dialog).toHaveAttribute('data-converge-preview', 'plan')
      await expect(dialog.locator('[data-check-badge]')).toHaveCount(0)
      await press(page, dialog.getByRole('switch', { name: 'Check only (dry run, changes nothing)' }), 'Space')
      await expect(dialog.locator('[data-check-badge]')).toBeVisible()
      const sent = backend.received.filter((c) => (c.type as string) === 'PreviewConverge') as unknown as { data: { check: boolean } }[]
      expect(sent.map((c) => c.data.check)).toEqual([false, true])
    })

    test('signed mode lists each step’s commit for review on the review page', async ({ page, backend }) => {
      const { converge } = converging(backend)
      converge.planFor = (t) =>
        convergePlan({
          binding_id: t.binding_id,
          hosts: [hostPlan('host-alpha', 'alpha', [{ job: 'debian' }, { job: 'keel-node' }])],
          review: [
            { job: 'debian', commit: 'a'.repeat(40), needs_review: false },
            { job: 'keel-node', commit: 'b'.repeat(40), needs_review: true },
          ],
        })
      await page.goto('/hosts')
      await page.getByRole('checkbox', { name: 'Select alpha' }).check()
      await page.locator('[data-converge-selected]').click()
      const review = preview(page).getByRole('region', { name: 'Review' })
      await expect(review).toContainText('1 of 2 steps need a reviewed commit')
      await expect(review.locator('[data-review="keel-node"]')).toHaveAttribute('data-needs-review', 'true')
      await expect(review.getByRole('link', { name: 'Open the review page' })).toHaveAttribute('href', '/reviews')
    })

    test('a host’s panel converges that host', async ({ page, backend }) => {
      converging(backend)
      await page.goto('/hosts')
      await page.getByRole('button', { name: 'Show alpha' }).click()
      const panel = page.getByRole('region', { name: 'Host alpha' })
      await expect(panel.getByRole('region', { name: 'Converge' })).toContainText('Not converged yet.')
      await press(page, panel.locator('[data-converge-host]'))
      expect((await backend.waitForCmd('PreviewConverge' as never)) as unknown).toMatchObject({
        data: { hosts: { kind: 'hosts', ids: ['host-alpha'] } },
      })
      await expect(preview(page).locator('[data-converge-confirm]')).toHaveText('Converge 1 host (2 steps)')
    })

    test('a viewer cannot select hosts to converge', async ({ page, backend }) => {
      converging(backend, { role: 'viewer' })
      await page.goto('/hosts')
      await expect(page.getByRole('button', { name: 'Show alpha' })).toBeVisible()
      await expect(page.getByRole('checkbox', { name: 'Select alpha' })).toHaveCount(0)
      await expect(page.locator('[data-converge-selected]')).toHaveCount(0)
    })

    test('Converge… is off, with the reason, when a selected host has a profile the operator lacks', async ({ page, backend }) => {
      const { ops } = converging(backend)
      backend.me = {
        ...(backend.me ?? fixtures.me()),
        grants: [{ workspace: ops.id, workspace_slug: 'ops', role: 'operator', profiles: ['base'] }],
      }
      await page.goto('/hosts')
      await page.getByRole('checkbox', { name: 'Select alpha' }).check()
      await expect(page.locator('[data-converge-selected]')).toBeDisabled()
      await expect(page.locator('#converge-blocked')).toHaveText('You can’t operate on every profile of alpha.')
      await page.getByRole('checkbox', { name: 'Select alpha' }).uncheck()
      await page.getByRole('checkbox', { name: 'Select charlie' }).check()
      await expect(page.locator('[data-converge-selected]')).toBeEnabled()
    })
  })
}

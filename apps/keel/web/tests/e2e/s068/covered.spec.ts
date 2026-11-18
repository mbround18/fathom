/**
 * Spec 068 T044 (FR-020, contracts/ui.md "Runs list"): a path-triggered run's host rows say when a
 * converge covered the host, with a link to that converge, and why a step was skipped; the header
 * names the vars files the run was narrowed by. At 360 px and 1280 px.
 */
import { expect, test } from '../../support/fake-backend'
import { runInfo } from '../../support/fake-runs'
import { NOW } from '../../support/fake-repos'
import { running } from './runs-setup'
import { WIDTHS } from './setup'

const row = (host: string, name: string) => ({
  host,
  name,
  refusal: null,
  started_at: NOW,
  ended_at: NOW,
  stats: null,
  first_failure: null,
})

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('a covered host links to its converge; a skipped one says why; the narrowing is named', async ({ page, backend }) => {
      const { runs } = running(backend)
      const ws = backend.me?.grants[0]?.workspace ?? ''
      runs.add(
        runInfo({
          id: 'run-push',
          workspace: ws,
          job_name: 'keel-node',
          state: 'succeeded',
          ended_at: NOW,
          hosts: ['alpha', 'bravo', 'charlie'],
          narrowed_by: ['inventory/group_vars/keel_nodes.yml'],
        }),
        [
          { ...row('host-alpha', 'alpha'), state: 'succeeded' },
          { ...row('host-bravo', 'bravo'), state: 'covered', covered_by: 'run-converge-1' },
          { ...row('host-charlie', 'charlie'), state: 'skipped', skip_reason: 'dependency_failed' },
        ],
      )
      await page.goto('/runs/run-push')
      await expect(page.getByRole('note', { name: 'Narrowed' })).toHaveText(
        'Only the hosts this vars file names: inventory/group_vars/keel_nodes.yml.',
      )
      const covered = page.locator('[data-covered-by="run-converge-1"]:visible')
      await expect(covered).toBeVisible()
      const link = covered.getByRole('link', { name: 'converge run-conv' })
      await expect(link).toHaveAttribute('href', '/runs/run-converge-1')
      await expect(page.getByText('an earlier step failed').first()).toBeVisible()

      await link.focus()
      await page.keyboard.press('Enter')
      await expect(page).toHaveURL(/\/runs\/run-converge-1(\?|$)/)
    })

    test('a run neither narrowed nor covered shows neither', async ({ page, backend }) => {
      const { runs } = running(backend)
      const ws = backend.me?.grants[0]?.workspace ?? ''
      runs.add(runInfo({ id: 'run-plain', workspace: ws, state: 'succeeded' }), [{ ...row('host-alpha', 'alpha'), state: 'succeeded' }])
      await page.goto('/runs/run-plain')
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
      await expect(page.locator('[data-narrowed-by]')).toHaveCount(0)
      await expect(page.locator('[data-covered-by]')).toHaveCount(0)
    })
  })
}

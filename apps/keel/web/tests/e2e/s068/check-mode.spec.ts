/**
 * Spec 068 T051 (FR-011, contracts/ui.md "Start dialog", "Runs list"): the Start dialog's Check
 * toggle sends `check: true` and the preview says it changes nothing; a check run carries the
 * `check` badge on the runs list and on its own page. At 360 px and 1280 px, keyboard only.
 */
import { expect, test } from '../../support/fake-backend'
import { runInfo } from '../../support/fake-runs'
import { change, openStart, running } from './runs-setup'
import { press, WIDTHS } from './setup'

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('Check only starts a dry run the preview names', async ({ page, backend }) => {
      const { r068 } = running(backend)
      const dialog = await openStart(page, 'upgrade')
      await expect(dialog.locator('[data-credentials]')).toHaveText('No credentials are sent to the hosts.')
      const toggle = dialog.getByRole('switch', { name: /Check only/ })
      await expect(toggle).not.toBeChecked()
      await press(page, toggle, 'Space')
      await expect(toggle).toBeChecked()
      await press(page, dialog.getByRole('button', { name: 'Review run' }))
      expect(await backend.waitForCmd('PreviewRunStart')).toMatchObject({ data: { job: 'job-upgrade', check: true } })

      const preview = change(page, 'Check upgrade')
      await expect(preview).toContainText('Check (ansible-playbook --check --diff, changes nothing)')
      await press(page, preview.getByRole('button', { name: 'Confirm' }))
      await expect(preview.getByRole('button', { name: 'Done' })).toBeVisible()
      expect(r068.started[0]?.check).toBe(true)
    })

    test('a check run shows the check badge on the list and its page', async ({ page, backend }) => {
      const { runs } = running(backend)
      const ws = backend.me?.grants[0]?.workspace ?? ''
      runs.add(runInfo({ id: 'run-check', workspace: ws, job_name: 'upgrade', state: 'succeeded', check: true }))
      runs.add(runInfo({ id: 'run-real', workspace: ws, job_name: 'debian', state: 'succeeded' }))
      await page.goto('/runs')
      const checkRow = page.locator('a[href="/runs/run-check"]:visible').first()
      await expect(checkRow).toBeVisible()
      await expect(page.locator('[data-check-badge]:visible')).toHaveCount(1)

      await page.goto('/runs/run-check')
      await expect(page.getByRole('heading', { level: 1 })).toContainText('upgrade')
      await expect(page.locator('[data-check-badge]:visible')).toHaveText('check')
      await page.goto('/runs/run-real')
      await expect(page.getByRole('heading', { level: 1 })).toContainText('debian')
      await expect(page.locator('[data-check-badge]')).toHaveCount(0)
    })
  })
}

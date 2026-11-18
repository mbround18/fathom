/**
 * Spec 068 T066 (contracts/ui.md "Start dialog"): a job's `choice` param is a select of the values
 * listed at the pinned commit, starting at its default; the value goes out in `PreviewRunStart`;
 * a choice that can't be listed says why and can't be picked; a value the server no longer allows
 * comes back `param_invalid`. Credentials the run is sent are named. At 360 px and 1280 px.
 */
import { expect, test } from '../../support/fake-backend'
import { choiceParam } from '../../support/fake-runs-068'
import { change, openStart, running } from './runs-setup'
import { press, WIDTHS } from './setup'

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('a choice param is a select of its listed values, sent with the run', async ({ page, backend }) => {
      const { r068 } = running(backend)
      r068.job('upgrade', {
        params: { app: choiceParam(['jellyfin', 'keel', 'nextcloud'], { default: 'keel' }) },
        credentials: ['registry-token'],
      })
      const dialog = await openStart(page, 'upgrade')
      const app = dialog.getByLabel('app')
      await expect(app).toHaveValue('keel')
      await expect(app.locator('option')).toHaveText(['Pick a value…', 'jellyfin', 'keel', 'nextcloud'])
      await expect(dialog.locator('[data-credentials]')).toHaveText('Credentials sent: registry-token')
      await app.selectOption('nextcloud')

      await press(page, dialog.getByRole('button', { name: 'Review run' }))
      expect(await backend.waitForCmd('PreviewRunStart')).toMatchObject({
        data: { job: 'job-upgrade', params: { app: 'nextcloud' }, check: false },
      })
      const preview = change(page, 'Run upgrade')
      await expect(preview).toContainText('app = nextcloud')
      await press(page, preview.getByRole('button', { name: 'Confirm' }))
      await expect(preview.getByRole('button', { name: 'Done' })).toBeVisible()
      expect(r068.started).toHaveLength(1)
    })

    test('a choice that cannot be listed says why and starts nothing', async ({ page, backend }) => {
      const { r068 } = running(backend)
      r068.job('upgrade', { params: { app: choiceParam(null, { unavailable: 'choice_from_missing' }) } })
      const dialog = await openStart(page, 'upgrade')
      await expect(dialog.getByLabel('app')).toBeDisabled()
      await expect(dialog).toContainText('Its folder is missing at this commit')
      await press(page, dialog.getByRole('button', { name: 'Review run' }))
      await expect(dialog).toBeVisible()
      expect(r068.started).toHaveLength(0)
    })

    test('a value the server no longer lists is refused with param_invalid', async ({ page, backend }) => {
      const { r068 } = running(backend)
      r068.job('upgrade', { params: { app: choiceParam(['jellyfin', 'keel']) } })
      const dialog = await openStart(page, 'upgrade')
      await dialog.getByLabel('app').selectOption('jellyfin')
      // The binding moved on: jellyfin is gone from composes/ at the commit a run would pin.
      r068.job('upgrade', { params: { app: choiceParam(['keel']) } })
      await press(page, dialog.getByRole('button', { name: 'Review run' }))
      const refused = change(page, 'Run upgrade')
      await expect(refused).toContainText('jellyfin is not an allowed value of app')
      await expect(refused.getByRole('button', { name: 'Dismiss' })).toBeVisible()
    })
  })
}

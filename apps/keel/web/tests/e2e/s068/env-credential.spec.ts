/**
 * Spec 068 T056 (FR-016): an `env_value` credential can be created from the Credentials form, which
 * says what one is; the value is sent as `PreviewCredentialCreate` and leaves the form. At 360 px
 * and 1280 px, keyboard only.
 */
import { expect, test } from '../../support/fake-backend'
import { running } from './runs-setup'
import { press, WIDTHS } from './setup'

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('an Environment value credential is offered and previewed', async ({ page, backend }) => {
      running(backend, { role: 'ws_admin' })
      await page.goto('/bindings')
      const form = page.getByRole('form', { name: 'New credential' })
      await form.getByLabel('Name').fill('registry-token')
      await form.getByLabel('Kind').selectOption({ label: 'Environment value' })
      await expect(form.locator('[data-kind-hint]')).toContainText('env:')
      await form.getByLabel('Value').fill('s3cr3t')
      await press(page, form.getByRole('button', { name: 'Preview credential' }))
      expect(await backend.waitForCmd('PreviewCredentialCreate')).toMatchObject({
        data: { name: 'registry-token', kind: 'env_value', value: 's3cr3t' },
      })
      await expect(form.getByLabel('Value')).toHaveValue('')
    })
  })
}

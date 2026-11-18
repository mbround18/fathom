/**
 * Spec 068 T035 (contracts/ui.md "Enroll", US2): "Converge after enrollment" where enrollment
 * tokens are created. On by default, naming the profiles it converges to; unticked, the token is
 * asked for with `auto_converge: false`. Add host says it converges too. At 360 px and 1280 px.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { converging, press, WIDTHS } from './setup'

async function enrollForm(page: Page) {
  await page.goto('/hosts')
  await page.locator('[data-enroll-open]').click()
  const form = page.getByRole('dialog').getByRole('form', { name: 'Enroll host' })
  await form.getByLabel('Inventory name').fill('delta')
  return form
}

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('on by default, naming the profiles, and sent as the default', async ({ page, backend }) => {
      const { hosts } = converging(backend)
      hosts.profiles = ['base', 'keel-node']
      const form = await enrollForm(page)
      const box = form.getByRole('checkbox', { name: 'Converge after enrollment' })
      await expect(box).toBeChecked()
      await form.getByRole('textbox', { name: 'Profiles' }).fill('keel-node')
      await form.getByRole('textbox', { name: 'Profiles' }).press('Enter')
      await expect(form.locator('[data-auto-converge]')).toContainText('runs the steps for base, keel-node on it')
      await press(page, page.getByRole('dialog').getByRole('button', { name: 'Continue' }))
      // Absent means yes: the command is the one before 068.
      const sent = (await backend.waitForCmd('PreviewHostEnroll')).data
      expect(sent).toMatchObject({ inventory_name: 'delta', profiles: ['base', 'keel-node'] })
      expect('auto_converge' in sent).toBe(false)
    })

    test('unticked, the token is asked for without converging', async ({ page, backend }) => {
      converging(backend)
      const form = await enrollForm(page)
      const box = form.getByRole('checkbox', { name: 'Converge after enrollment' })
      await press(page, box, 'Space')
      await expect(box).not.toBeChecked()
      await expect(form.locator('[data-auto-converge]')).toContainText('Keel waits for someone to converge it.')
      await press(page, page.getByRole('dialog').getByRole('button', { name: 'Continue' }))
      expect((await backend.waitForCmd('PreviewHostEnroll')).data).toMatchObject({
        inventory_name: 'delta',
        profiles: ['base'],
        auto_converge: false,
      })
    })

    test('Add host says the host converges once it enrolls', async ({ page, backend }) => {
      converging(backend, { role: 'ws_admin' })
      await page.goto('/hosts')
      await page.getByRole('button', { name: 'Add host' }).click()
      const dialog = page.getByRole('dialog', { name: 'Add host' })
      await expect(dialog.locator('[data-add-converge]')).toContainText('converges it to these profiles once it enrolls')
    })
  })
}

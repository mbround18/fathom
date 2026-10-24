/**
 * Spec 053, the volume explorer: the Files tab on the stack page. Later tasks add the browse,
 * edit, transfer, manage and history specs beside this one.
 */
import { expect, fixtures as f, test } from '../../support/fake-backend'

test('the stack page opens', async ({ page, backend }) => {
  backend.stacks = f.stackList([f.stack('web')])
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
})

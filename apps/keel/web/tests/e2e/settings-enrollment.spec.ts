/**
 * Spec 060 US2 (T028): System Settings → Enrollment. The snippet template is checked as it is
 * typed and previewed live; the instructions are Markdown with raw HTML shown as text; Save goes
 * through step-up; a save by someone else in between shows the conflict banner and overwrites
 * nothing. Against the fake backend (tests/support/fake-settings.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../support/fake-backend'
import { governance } from '../support/fake-governance'
import { DEFAULT_TEMPLATE, FakeSettings } from '../support/fake-settings'

function owner(backend: FakeBackend): FakeSettings {
  backend.me = fixtures.me({ instance_role: 'owner', grants: [], verification: fixtures.verification({ verified: true }) })
  governance(backend)
  return new FakeSettings(backend)
}

test('the template is checked inline, previewed live and saved with step-up', async ({ page, backend }) => {
  const settings = owner(backend)
  await page.goto('/settings')
  // Workspace is the first tab since spec 066 (R10).
  await expect(page).toHaveURL(/\/settings\/workspace(\?ws=[^#]*)?$/)
  await page.getByRole('tab', { name: 'Enrollment' }).click()
  await expect(page).toHaveURL(/\/settings\/enrollment(\?ws=[^#]*)?$/)

  const form = page.getByRole('form', { name: 'Enrollment settings' })
  const template = form.getByRole('textbox', { name: 'Snippet template' })
  await expect(template).toHaveValue(DEFAULT_TEMPLATE)
  await expect(form.getByText('Default', { exact: true })).toBeVisible()
  const preview = form.locator('[data-template-preview]')
  await expect(preview).toContainText('keel-enroll-v1.')
  await expect(preview).toContainText('make keel-node')

  // Inline errors, in the server's order, and Save held back while one shows.
  const save = form.getByRole('button', { name: 'Save' })
  await template.fill('curl https://keel.example.com/join | sh')
  await expect(form.locator('[data-template-error]')).toHaveAttribute('data-template-error', 'missing_token')
  await expect(template).toHaveAttribute('aria-invalid', 'true')
  await expect(save).toBeDisabled()
  await template.fill('KEEL_TOKEN=%t %q')
  await expect(form.locator('[data-template-error]')).toHaveAttribute('data-template-error', 'unknown_placeholder')
  await template.fill('x'.repeat(4097) + '%t')
  await expect(form.locator('[data-template-error]')).toHaveAttribute('data-template-error', 'too_long')

  // A valid one: the preview follows every keystroke, with the placeholders filled.
  await template.fill('KEEL_TOKEN=%t keel join %w %n (%p) 100%%')
  await expect(form.locator('[data-template-error]')).toHaveCount(0)
  await expect(preview).toContainText('keel join ops web-01 (base, web) 100%')
  await expect(preview).not.toContainText('%t')

  // Instructions: Markdown in Preview, with raw HTML shown as the characters it is.
  await form.getByRole('tab', { name: 'Write' }).click()
  await form.getByRole('textbox', { name: 'Instructions' }).fill('Open **port 7443** first.\n\n<script>window.pwned = 1</script>')
  await form.getByRole('tab', { name: 'Preview' }).click()
  await expect(form.locator('strong')).toHaveText('port 7443')
  await expect(form).toContainText('<script>window.pwned = 1</script>')
  expect(await page.evaluate(() => (window as unknown as { pwned?: number }).pwned)).toBeUndefined()

  // Save: previewed, confirmed with the session's fresh passkey check.
  await save.click()
  expect((await backend.waitForCmd('PreviewSettingsUpdate')).data).toEqual({
    template: 'KEEL_TOKEN=%t keel join %w %n (%p) 100%%',
    instructions: 'Open **port 7443** first.\n\n<script>window.pwned = 1</script>',
    version: 0,
  })
  const dialog = page.getByRole('dialog')
  await expect(dialog.locator('[data-step-up]')).toBeVisible()
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  expect(settings.template).toBe('KEEL_TOKEN=%t keel join %w %n (%p) 100%%')
  expect(settings.version).toBe(1)
  await expect(template).toHaveValue('KEEL_TOKEN=%t keel join %w %n (%p) 100%%')
  await expect(form.getByText('Default', { exact: true })).toHaveCount(0)
  await expect(save).toBeDisabled()
})

test('a save after someone else saved shows the conflict banner and overwrites nothing', async ({ page, backend }) => {
  const settings = owner(backend)
  await page.goto('/settings/enrollment')
  const form = page.getByRole('form', { name: 'Enrollment settings' })
  const template = form.getByRole('textbox', { name: 'Snippet template' })
  await expect(template).toHaveValue(DEFAULT_TEMPLATE)

  settings.savedElsewhere('echo theirs %t')
  await template.fill('echo mine %t')
  await form.getByRole('button', { name: 'Save' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await expect(dialog).toContainText('changed since you loaded them')
  await dialog.getByRole('button', { name: 'Dismiss' }).click()

  const banner = page.locator('[data-conflict]')
  await expect(banner).toContainText('Someone else saved changes to these settings')
  expect(settings.template).toBe('echo theirs %t')

  await banner.getByRole('button', { name: 'Reload' }).click()
  await expect(banner).toHaveCount(0)
  await expect(template).toHaveValue('echo theirs %t')
})

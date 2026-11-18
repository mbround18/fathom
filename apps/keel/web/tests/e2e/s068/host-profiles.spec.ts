/**
 * Spec 068 T040 (contracts/ui.md "Host page"): a host's Profiles card (chips, the ceiling, the
 * outside-ceiling marker in words), and editing them: the editor, its preview (change, outside
 * the ceiling, principals after, converge steps after), Confirm by keyboard, a refusal, and the
 * passkey when the session's check has lapsed. At 360 px and 1280 px, against the fake backend.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test } from '../../support/fake-backend'
import { FakeProfiles } from '../../support/fake-profiles'
import { virtualAuthenticator } from '../../support/webauthn'
import { registerPasskey } from '../s061/support'
import { converging, press, WIDTHS } from './setup'

async function openHost(page: Page, name: string) {
  await page.goto('/hosts')
  await page.getByRole('button', { name: `Show ${name}` }).click()
  const panel = page.getByRole('region', { name: `Host ${name}` })
  await expect(panel).toBeVisible()
  return panel
}

function editor(page: Page, name: string) {
  return page.getByRole('dialog', { name: `Profiles of ${name}` })
}

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('the card marks a profile outside the ceiling in words', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'ws_admin' })
      new FakeProfiles(backend, hosts)
      const panel = await openHost(page, 'alpha')
      const card = panel.getByRole('region', { name: 'Profiles' })
      await expect(card.locator('[data-profile]')).toHaveText(['base', /keel-node/])
      // alpha's runner accepts only base: keel-node carries the icon and the words, not just colour.
      await expect(card.locator('[data-profile="keel-node"]')).toHaveAttribute('data-outside-ceiling', 'true')
      await expect(card.locator('[data-profile="keel-node"]')).toContainText('outside ceiling')
      await expect(card.locator('[data-profile="base"]')).not.toContainText('outside ceiling')
      await expect(card.locator('[data-ceiling]')).toHaveText('This host accepts: base.')
      await expect(card.locator('[data-outside-note]')).toContainText('keel_runner_allowed_profiles')
    })

    test('editing previews the change and Confirm by keyboard sets the profiles', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'ws_admin' })
      const profiles = new FakeProfiles(backend, hosts)
      hosts.hosts = hosts.hosts.map((h) =>
        h.id === 'host-charlie' ? { ...h, allowed_profiles: ['base', 'keel-node', 'debian'] } : h,
      )
      const panel = await openHost(page, 'charlie')
      await press(page, panel.locator('[data-edit-profiles]'))

      const dialog = editor(page, 'charlie')
      await expect(dialog).toHaveAttribute('data-profiles-dialog', 'edit')
      await expect(dialog.locator('[data-profiles-preview]')).toBeDisabled()
      const input = dialog.getByRole('textbox', { name: 'Profiles' })
      await input.fill('keel-node')
      await input.press('Enter')
      await input.fill('web')
      await input.press('Enter')
      await expect(dialog.locator('[data-change="add"]')).toHaveText(['Addkeel-node', 'Addweb'])
      // web is outside charlie's ceiling: said before the preview, too.
      await expect(dialog.locator('[data-outside-warning]')).toContainText('web')
      await press(page, dialog.locator('[data-profiles-preview]'))

      expect((await backend.waitForCmd('PreviewHostProfiles')).data).toEqual({
        host_id: 'host-charlie',
        add: ['keel-node', 'web'],
        remove: [],
      })
      await expect(dialog).toHaveAttribute('data-profiles-dialog', 'preview')
      await expect(dialog.locator('[data-preview-outside]')).toContainText('web')
      await expect(dialog.getByRole('list', { name: 'Principals after' })).toContainText('profile:keel-node')
      await expect(dialog.getByRole('list', { name: 'Steps after' }).getByRole('listitem')).toHaveText([
        'base (from base)',
        'keel-node (from keel-node)',
        'web (from web)',
      ])
      await expect(dialog.locator('[data-step-up]')).toHaveText('Needs your passkey.')

      await press(page, dialog.locator('[data-profiles-confirm]'))
      await expect(dialog).toBeHidden()
      expect(profiles.previewed).toHaveLength(1)
      const card = panel.getByRole('region', { name: 'Profiles' })
      await expect(card.locator('[data-profile]')).toHaveText(['base', 'keel-node', /web/])
      await expect(card.locator('[data-profile="web"]')).toContainText('outside ceiling')
    })

    test('a refusal is shown in the server’s words, and Back returns to the editor', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'ws_admin' })
      const profiles = new FakeProfiles(backend, hosts)
      profiles.known = ['base', 'keel-node']
      const panel = await openHost(page, 'charlie')
      await panel.locator('[data-edit-profiles]').click()
      const dialog = editor(page, 'charlie')
      await dialog.getByRole('textbox', { name: 'Profiles' }).fill('nginx')
      await dialog.getByRole('textbox', { name: 'Profiles' }).press('Enter')
      await dialog.locator('[data-profiles-preview]').click()
      await expect(dialog).toHaveAttribute('data-profiles-dialog', 'refused')
      await expect(dialog).toContainText('No repository defines the profile nginx.')
      await dialog.getByRole('button', { name: 'Back' }).click()
      await expect(dialog).toHaveAttribute('data-profiles-dialog', 'edit')
      await expect(dialog.locator('[data-profile-chip]')).toHaveCount(2)
    })

    test('an operator sees the profiles but cannot edit them', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'operator' })
      new FakeProfiles(backend, hosts)
      const panel = await openHost(page, 'alpha')
      await expect(panel.getByRole('region', { name: 'Profiles' }).locator('[data-profile]')).toHaveCount(2)
      await expect(panel.locator('[data-edit-profiles]')).toHaveCount(0)
    })
  })
}

test('Confirm asks for a passkey when the check has lapsed; cancelling it sends nothing', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  const { hosts } = converging(backend, { role: 'ws_admin' })
  new FakeProfiles(backend, hosts)
  const panel = await openHost(page, 'charlie')
  await registerPasskey(page)
  const verification = fixtures.verification({ enrolled: true, verified: false })
  backend.me = { ...(backend.me ?? fixtures.me()), verification }
  backend.push({ type: 'Verification', data: verification })

  await panel.locator('[data-edit-profiles]').click()
  const dialog = editor(page, 'charlie')
  await dialog.getByRole('textbox', { name: 'Profiles' }).fill('keel-node')
  await dialog.getByRole('textbox', { name: 'Profiles' }).press('Enter')
  await dialog.locator('[data-profiles-preview]').click()
  await expect(dialog).toHaveAttribute('data-profiles-dialog', 'preview')
  await dialog.locator('[data-profiles-confirm]').click()

  const stepUp = page.getByRole('dialog', { name: 'Verify it’s you' })
  await expect(stepUp).toBeVisible()
  await stepUp.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toHaveAttribute('data-profiles-dialog', 'preview')
  expect(backend.received.some((c) => c.type === 'Confirm')).toBe(false)

  await dialog.locator('[data-profiles-confirm]').click()
  await stepUp.getByRole('button', { name: 'Use passkey' }).click()
  // The editor steps aside while the passkey prompt is up, so wait for the command itself.
  expect((await backend.waitForCmd('Confirm')).data.step_up).toBeTruthy()
  await expect(stepUp).toBeHidden()
  await expect(dialog).toBeHidden()
  await expect(panel.getByRole('region', { name: 'Profiles' }).locator('[data-profile]')).toHaveText(['base', /keel-node/])
})

/**
 * Spec 068 T040 (contracts/ui.md "Host page", "Hosts"): the inventory's profile suggestions. The
 * Hosts list's dot and its tooltip, the host's suggestions with Accept (the editor's preview,
 * pre-filled) and Dismiss, a sync that moves them while the page is open, who may act on them,
 * and the card with converge off. At 360 px and 1280 px, against the fake backend.
 */
import type { Page } from '@playwright/test'

import { expect, test } from '../../support/fake-backend'
import { FakeProfiles, suggestion } from '../../support/fake-profiles'
import { converging, press, WIDTHS } from './setup'

function dot(page: Page, name: string) {
  return page.getByRole('button', { name: new RegExp(`^${name}: \\d+ suggested from inventory groups$`) })
}

function suggestions(page: Page, name: string) {
  return page.getByRole('region', { name: `Host ${name}` }).getByRole('region', { name: 'Inventory suggestions' })
}

for (const size of WIDTHS) {
  test.describe(size.name, () => {
    test.use({ viewport: { width: size.width, height: size.height } })

    test('the dot opens the host, and Accept previews the suggestion before setting it', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'ws_admin' })
      const profiles = new FakeProfiles(backend, hosts)
      profiles.seed([
        suggestion({ host_id: 'host-alpha', profile: 'k3s-agent', groups: ['k3s_agents'] }),
        suggestion({ host_id: 'host-alpha', profile: 'keel-node', action: 'remove', groups: [] }),
      ])
      await page.goto('/hosts')
      await expect(dot(page, 'alpha')).toHaveAccessibleName('alpha: 2 suggested from inventory groups')
      await expect(dot(page, 'bravo')).toHaveCount(0)
      if (size.name === 'desktop') {
        await dot(page, 'alpha').hover()
        await expect(page.getByRole('tooltip')).toHaveText('2 suggested from inventory groups')
      }
      await press(page, dot(page, 'alpha'))

      const list = suggestions(page, 'alpha')
      await expect(list.locator('[data-suggestion]')).toHaveCount(2)
      await expect(list.locator('[data-suggestion="k3s-agent"]')).toContainText('from k3s_agents')
      await expect(list.locator('[data-suggestion="keel-node"]')).toContainText('no group maps to it')
      await press(page, list.locator('[data-suggestion="k3s-agent"] [data-suggestion-accept]'))

      expect((await backend.waitForCmd('PreviewHostProfiles')).data).toEqual({
        host_id: 'host-alpha',
        add: ['k3s-agent'],
        remove: [],
      })
      const dialog = page.getByRole('dialog', { name: 'Profiles of alpha' })
      await expect(dialog).toHaveAttribute('data-profiles-dialog', 'preview')
      await expect(dialog.locator('[data-change="add"]')).toHaveText('Addk3s-agent')
      await press(page, dialog.locator('[data-profiles-confirm]'))
      await expect(dialog).toBeHidden()

      // The change settled that suggestion; the other stays, and the dot counts it.
      await expect(list.locator('[data-suggestion]')).toHaveCount(1)
      await expect(dot(page, 'alpha')).toHaveAccessibleName('alpha: 1 suggested from inventory groups')
    })

    test('Dismiss drops a suggestion, and the dot goes with the last one', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'ws_admin' })
      const profiles = new FakeProfiles(backend, hosts)
      profiles.seed([suggestion({ host_id: 'host-bravo', profile: 'k3s-agent' })])
      await page.goto('/hosts')
      await dot(page, 'bravo').click()
      const list = suggestions(page, 'bravo')
      await press(page, list.getByRole('button', { name: 'Dismiss k3s-agent' }))
      await expect(list).toHaveCount(0)
      await expect(dot(page, 'bravo')).toHaveCount(0)
      expect(profiles.dismissed.map((s) => s.profile)).toEqual(['k3s-agent'])
      expect(backend.received.some((c) => c.type === 'PreviewHostProfiles')).toBe(false)
    })

    test('a sync that moves suggestions shows them without a reload', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'ws_admin' })
      const profiles = new FakeProfiles(backend, hosts)
      await page.goto('/hosts')
      await page.getByRole('button', { name: 'Show charlie' }).click()
      await expect(dot(page, 'charlie')).toHaveCount(0)
      profiles.sync([suggestion({ host_id: 'host-charlie', profile: 'web', groups: ['webservers'] })])
      await expect(dot(page, 'charlie')).toHaveAccessibleName('charlie: 1 suggested from inventory groups')
      await expect(suggestions(page, 'charlie').locator('[data-suggestion="web"]')).toContainText('from webservers')
    })

    test('an operator sees suggestions but cannot act on them', async ({ page, backend }) => {
      const { hosts } = converging(backend, { role: 'operator' })
      const profiles = new FakeProfiles(backend, hosts)
      profiles.seed([suggestion({ host_id: 'host-alpha', profile: 'k3s-agent' })])
      await page.goto('/hosts')
      await dot(page, 'alpha').click()
      const list = suggestions(page, 'alpha')
      await expect(list.locator('[data-suggestion="k3s-agent"]')).toBeVisible()
      await expect(list.locator('[data-suggestion-accept], [data-suggestion-dismiss]')).toHaveCount(0)
    })
  })
}

test('with converge off the card shows the profiles alone', async ({ page, backend }) => {
  const { hosts } = converging(backend, { role: 'ws_admin' })
  const profiles = new FakeProfiles(backend, hosts)
  profiles.seed([suggestion({ host_id: 'host-alpha', profile: 'k3s-agent' })])
  profiles.disabled = true
  await page.goto('/hosts')
  await page.getByRole('button', { name: 'Show alpha' }).click()
  const card = page.getByRole('region', { name: 'Host alpha' }).getByRole('region', { name: 'Profiles' })
  await expect(card.locator('[data-profile]')).toHaveCount(2)
  await expect(card.locator('[data-edit-profiles]')).toHaveCount(0)
  await expect(card.getByRole('region', { name: 'Inventory suggestions' })).toHaveCount(0)
})

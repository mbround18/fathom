/**
 * Spec 067 US3 (T036): Settings → Relays. Register with the fingerprint in the preview, revoke
 * naming the hosts it cuts off, a quarantined relay's reason and Release, zones, and Keel's
 * relay-link key: copy, rotate, activate. Against the fake backend (tests/support/fake-relays.ts).
 */
import { expect, test } from '../../support/fake-backend'
import { fingerprintOf, LINK_KEY_LINE, RELAY_KEY } from '../../support/fake-relays'
import { confirmChange } from '../s066/repo-setup'

import { scene } from './scene'

test('a relay is registered, with the fingerprint Keel will pin in the preview', async ({ page, backend }) => {
  scene(backend)
  await page.goto('/settings/relays')
  const relays = page.getByRole('table', { name: 'Relays' })
  await expect(relays.getByRole('row', { name: /gw-lab/ })).toContainText('connected')

  await page.getByRole('button', { name: 'Register relay' }).click()
  const form = page.getByRole('dialog', { name: 'Register relay' })
  await form.getByLabel('Name').fill('gw-lab-2')
  await form.getByLabel('Zone').selectOption({ label: 'lab' })
  await form.getByLabel('Dial address').fill('https://gw2')
  await expect(form).toContainText('host:port, with no scheme.')
  await expect(form.getByRole('button', { name: 'Preview' })).toBeDisabled()
  await form.getByLabel('Dial address').fill('gw2.lab.example.net:7444')
  await form.getByLabel('Host key').fill(RELAY_KEY)
  await form.getByLabel('Runs on host (optional)').selectOption({ label: 'gate-1' })
  await form.getByRole('button', { name: 'Preview' }).click()

  const dialog = page.getByRole('dialog', { name: 'Register relay gw-lab-2' })
  await expect(dialog).toContainText(`pinning host key ${fingerprintOf(RELAY_KEY)}`)
  await expect(dialog.getByRole('list', { name: 'Warnings' })).toContainText(LINK_KEY_LINE)
  const sent = await backend.waitForCmd('PreviewRelayRegister')
  expect(sent.data).toMatchObject({ name: 'gw-lab-2', zone: 'zone-lab', dial_address: 'gw2.lab.example.net:7444', host: 'host-0002' })
  await confirmChange(page, 'Register relay gw-lab-2')

  await expect(form).toBeHidden()
  await expect(relays.getByRole('row', { name: /gw-lab-2/ })).toContainText('connecting')
})

test('revoke is one click to a preview naming the hosts it cuts off', async ({ page, backend }) => {
  scene(backend)
  await page.goto('/settings/relays')
  await page.getByRole('button', { name: 'Revoke relay gw-lab' }).click()
  const dialog = page.getByRole('dialog', { name: 'Revoke relay gw-lab' })
  await expect(dialog).toContainText('their runs in progress are lost: lab-1, lab-2')
  await expect(dialog.getByRole('list', { name: 'Affected' })).toContainText('lab-1')
  await expect(dialog).not.toContainText('passkey')
  await confirmChange(page, 'Revoke relay gw-lab')

  const row = page.getByRole('table', { name: 'Relays' }).getByRole('row', { name: /gw-lab/ })
  await expect(row).toHaveAttribute('data-relay-state', 'revoked')
  await expect(row.getByRole('button', { name: 'Revoke relay gw-lab' })).toHaveCount(0)
})

test('a quarantined relay shows why, and is released through a preview', async ({ page, backend }) => {
  const { relays } = scene(backend)
  relays.relay('gw-bad', 'dmz', { state: 'quarantined', last_error: 'opened a stream Keel did not ask for', streams: 0, rtt_ms: null })
  await page.goto('/settings/relays')
  const row = page.getByRole('table', { name: 'Relays' }).getByRole('row', { name: /gw-bad/ })
  await expect(row).toContainText('quarantined')
  await expect(row).toContainText('opened a stream Keel did not ask for')

  await row.getByRole('button', { name: 'Release relay gw-bad' }).click()
  const dialog = page.getByRole('dialog', { name: 'Release relay gw-bad' })
  await expect(dialog.getByRole('list', { name: 'Warnings' })).toContainText('Release it only once you know why it did.')
  await confirmChange(page, 'Release relay gw-bad')
  await expect(row).toHaveAttribute('data-relay-state', 'connecting')
})

test('zones: create one, and a zone in use lists its blockers', async ({ page, backend }) => {
  scene(backend)
  await page.goto('/settings/relays')
  const zones = page.getByRole('table', { name: 'Zones' })
  await expect(zones.getByRole('row', { name: /lab/ })).toContainText('2')

  await page.getByRole('button', { name: 'Create zone' }).click()
  const form = page.getByRole('dialog', { name: 'Create zone' })
  await form.getByLabel('Name').fill('direct')
  await expect(form).toContainText('not “direct”')
  await form.getByLabel('Name').fill('edge')
  await form.getByRole('button', { name: 'Preview' }).click()
  await expect(page.getByRole('dialog', { name: 'Create zone edge' })).toContainText('register a relay for it before moving hosts in')
  await confirmChange(page, 'Create zone edge')
  await expect(zones.getByRole('row', { name: /edge/ })).toBeVisible()

  await page.getByRole('button', { name: 'Remove zone lab' }).click()
  const remove = page.getByRole('dialog', { name: 'Remove zone lab' })
  await expect(remove.getByRole('list', { name: 'Warnings' })).toContainText('In use by host lab-1, host lab-2, relay gw-lab.')
  await remove.getByRole('button', { name: 'Confirm' }).click()
  await expect(remove).toContainText('in use')
})

test("Keel's relay-link key is copied, rotated and activated by an instance admin", async ({ page, context, backend }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  scene(backend)
  await page.goto('/settings/relays')
  const section = page.getByRole('region', { name: 'Relay-link key' })
  await expect(section).toContainText('Add this to keel_relay_hub_keys')
  await section.getByRole('button', { name: 'Copy relay-link key' }).click()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(LINK_KEY_LINE)

  await section.getByRole('button', { name: 'Rotate' }).click()
  await confirmChange(page, 'Rotate relay-link key')
  await expect(section.locator('[data-link-key="pending"]')).toBeVisible()
  await section.getByRole('button', { name: 'Activate' }).click()
  await expect(page.getByRole('dialog', { name: 'Activate relay-link key' }).getByRole('list', { name: 'Warnings' })).toContainText(
    'Relays: gw-lab, gw-dmz.',
  )
  await confirmChange(page, 'Activate relay-link key')
  await expect(section.locator('[data-link-key="pending"]')).toHaveCount(0)
  await expect(section.getByRole('list', { name: 'Relay-link keys' })).toContainText('retired')
})

test('a workspace admin manages relays but does not see the link key', async ({ page, backend }) => {
  scene(backend, { role: 'ws_admin' })
  await page.goto('/settings/relays')
  await expect(page.getByRole('table', { name: 'Relays' }).getByRole('row', { name: /gw-lab/ })).toBeVisible()
  await expect(page.locator('[data-link-key="hidden"]')).toContainText('Registering a relay shows the line in its preview.')
  await expect(page.getByRole('button', { name: 'Register relay' })).toBeEnabled()
  expect(backend.received.some((c) => c.type === 'GetRelayLinkKeys')).toBe(false)
})

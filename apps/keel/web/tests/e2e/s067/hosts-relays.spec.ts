/**
 * Spec 067 US3 (T036): the Hosts page grouped by zone, each host's path, the relay chips, a relay
 * dropping (only its zone flips), Change zone, and the run preview's warning about a relay host.
 * Against the fake backend (tests/support/fake-relays.ts).
 */
import { expect, test } from '../../support/fake-backend'
import { confirmChange } from '../s066/repo-setup'

import { scene } from './scene'

test('hosts are grouped by zone, direct first, each with its path and its relays', async ({ page, backend }) => {
  scene(backend)
  await page.goto('/hosts')

  await expect(page.locator('section[data-zone]')).toHaveCount(3)
  const zones = page.locator('section[data-zone]')
  await expect(zones.nth(0)).toHaveAttribute('data-zone', 'direct')
  await expect(zones.nth(1)).toHaveAttribute('data-zone', 'dmz')
  await expect(zones.nth(2)).toHaveAttribute('data-zone', 'lab')

  const direct = page.getByRole('table', { name: 'Hosts in direct' })
  await expect(direct.getByRole('row', { name: /gate-1/ })).toContainText('direct')
  const lab = page.getByRole('table', { name: 'Hosts in lab' })
  await expect(lab.getByRole('row', { name: /lab-1/ })).toContainText('via gw-lab')
  await expect(lab.getByRole('row', { name: /lab-2/ })).toContainText('via gw-lab')
  await expect(page.getByRole('table', { name: 'Hosts in dmz' }).getByRole('row', { name: /dmz-1/ })).toContainText('via gw-dmz')

  // The Path column sits after State and before the versions.
  await expect(lab.getByRole('columnheader')).toHaveText(['Name', 'State', 'Path', 'Profiles', 'Runner', 'Ansible', 'Last seen', /.*/])

  // Relay chips: state, and RTT and streams at desktop width.
  const chips = page.getByRole('list', { name: 'Relays for lab' })
  const chip = chips.locator('[data-relay="gw-lab"]')
  await expect(chip).toHaveAttribute('data-relay-state', 'connected')
  await expect(chip).toContainText('9 ms · 2 streams')
  await expect(page.getByRole('list', { name: 'Relays for direct' })).toHaveCount(0)
})

test('a relay going unreachable flips only its zone', async ({ page, backend }) => {
  const { relays } = scene(backend)
  await page.goto('/hosts')
  const lab = page.getByRole('table', { name: 'Hosts in lab' })
  await expect(lab.getByRole('row', { name: /lab-1/ })).toContainText('via gw-lab')

  relays.setState('gw-lab', 'unreachable', 'connection refused')

  await expect(page.locator('[data-relay="gw-lab"]')).toHaveAttribute('data-relay-state', 'unreachable')
  await expect(lab.getByRole('row', { name: /lab-1/ })).toContainText('disconnected (relay unreachable)')
  await expect(lab.getByRole('row', { name: /lab-2/ })).toContainText('disconnected (relay unreachable)')
  await expect(lab.getByRole('row', { name: /lab-1/ })).toHaveAttribute('data-host-state', 'offline')
  // Nothing else moved.
  await expect(page.locator('[data-relay="gw-dmz"]')).toHaveAttribute('data-relay-state', 'connected')
  await expect(page.getByRole('table', { name: 'Hosts in dmz' }).getByRole('row', { name: /dmz-1/ })).toContainText('via gw-dmz')
  await expect(page.getByRole('table', { name: 'Hosts in direct' }).getByRole('row', { name: /gate-1/ })).toContainText('direct')

  // Back up: the zone's hosts come back through it.
  relays.setState('gw-lab', 'connected')
  await expect(lab.getByRole('row', { name: /lab-1/ })).toContainText('via gw-lab')
})

test('on a phone, a relay chip keeps to its name and state', async ({ page, backend }) => {
  scene(backend)
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/hosts')
  const chip = page.locator('[data-relay="gw-lab"]')
  await expect(chip).toContainText('gw-lab')
  await expect(chip).toContainText('connected')
  await expect(chip.getByText('2 streams')).toBeHidden()
  // The card list says the path too.
  await expect(page.getByRole('list', { name: 'Hosts in lab' })).toContainText('via gw-lab')
  // No sideways scroll.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
})

test('Change zone moves a host through preview and confirm, and warns about a zone with no relay', async ({ page, backend }) => {
  const { relays } = scene(backend)
  relays.zone('empty')
  await page.goto('/hosts')

  await page.getByRole('button', { name: 'Show gate-1' }).click()
  const inspector = page.getByRole('region', { name: 'Host gate-1' })
  await expect(inspector.locator('[data-host-zone]')).toContainText('connected · direct')

  // A zone with no relay: the preview says the host would have no path.
  await inspector.getByLabel('New zone for gate-1').selectOption({ label: 'empty' })
  await inspector.getByRole('button', { name: 'Change zone' }).click()
  const dialog = page.getByRole('dialog', { name: 'Change zone of gate-1' })
  await expect(dialog.getByRole('list', { name: 'Warnings' })).toContainText('Zone empty has no relay')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(dialog).toBeHidden()

  await inspector.getByLabel('New zone for gate-1').selectOption({ label: 'dmz' })
  await inspector.getByRole('button', { name: 'Change zone' }).click()
  await expect(dialog).toContainText('Move host gate-1 from zone direct to zone dmz')
  expect(backend.received.filter((c) => c.type === 'PreviewHostZone').map((c) => c.data)).toEqual([
    { host: 'host-0002', zone: 'zone-empty' },
    { host: 'host-0002', zone: 'zone-dmz' },
  ])
  await confirmChange(page, 'Change zone of gate-1')

  await expect(page.getByRole('table', { name: 'Hosts in dmz' }).getByRole('row', { name: /gate-1/ })).toContainText('via gw-dmz')
  await expect(inspector.locator('[data-host-zone]')).toContainText('dmz')
})

test('a run that includes the only relay of a zone warns before it starts', async ({ page, backend }) => {
  scene(backend)
  await page.goto('/repository?tab=jobs&item=debian')
  await page.getByRole('button', { name: 'Run', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: 'Run debian' })
  const warnings = dialog.getByRole('list', { name: 'Warnings' })
  await expect(warnings).toContainText('This run includes gate-1, which runs relay gw-lab, the only relay for lab-1, lab-2.')
  await expect(warnings).toContainText('Consider running gate-1 separately.')
})

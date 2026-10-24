/**
 * Spec 052 US3 (T021): an administrator sees one person's sessions and networks on the Users page,
 * whitelists a network from a trusted network, removes it (back to unknown) and forgets it, and
 * ends every session. From an untrusted network the server refuses whitelisting and audits it.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const BOB = 'github:2002'

function seed(backend: FakeBackend): void {
  backend.users = {
    users: [f.user('github:1001', { username: 'ada', role: 'admin', owner: true }), f.user(BOB, { username: 'bob', stacks: [] })],
    pending: [],
  }
  backend.userSessions[BOB] = [
    f.sessionView('bob1', { browser: 'Safari on iOS' }),
    f.sessionView('bob2', { browser: 'Firefox on Linux', address: '198.51.100.9', network: '198.51.100.9/32', trusted: false }),
  ]
  backend.userNetworks[BOB] = [f.networkView('203.0.113.7/32', { source: 'first', trusted: true, reason: 'first' })]
}

async function openPanel(page: Page) {
  await page.goto('/admin/users')
  await page.locator(`[data-user="${BOB}"]`).getByRole('button', { name: 'Sessions and networks for bob' }).click()
  const panel = page.getByRole('region', { name: 'Sessions and networks for bob' })
  await expect(panel.getByRole('list', { name: 'Sessions for bob' }).locator('[data-session]')).toHaveCount(2)
  return panel
}

test('an admin whitelists a network, removes it back to unknown, then forgets it', async ({ page, backend }) => {
  seed(backend)
  const panel = await openPanel(page)
  const nets = panel.getByRole('list', { name: 'Networks for bob' })

  await panel.getByRole('textbox', { name: 'Network address' }).fill('198.51.100.0/33')
  await panel.getByRole('button', { name: 'Whitelist' }).click()
  await expect(panel.getByRole('alert')).toBeVisible()
  expect(backend.http.filter((h) => h.path.endsWith('/whitelist'))).toHaveLength(0)

  await panel.getByRole('textbox', { name: 'Network address' }).fill('198.51.100.0/24')
  await panel.getByRole('textbox', { name: 'Label' }).fill('Office')
  await panel.getByRole('button', { name: 'Whitelist' }).click()
  const added = nets.locator('[data-network="198.51.100.0/24"]')
  await expect(added).toHaveAttribute('data-reason', 'whitelist')
  await expect(added.locator('[data-field="trust"]')).toHaveAttribute('data-trusted', 'true')
  await expect(added.locator('[data-field="label"]')).toHaveText('“Office”')
  await expect(panel.getByRole('textbox', { name: 'Network address' })).toHaveValue('')

  await nets.getByRole('button', { name: 'Remove whitelist 198.51.100.0/24' }).click()
  await expect(added).toHaveAttribute('data-reason', 'unknown')
  await expect(added.locator('[data-field="trust"]')).toHaveAttribute('data-trusted', 'false')

  await nets.getByRole('button', { name: 'Forget 198.51.100.0/24' }).click()
  await expect(added).toHaveCount(0)
  expect(backend.audit.map((e) => e.action).filter((a) => a.startsWith('network_'))).toEqual(
    expect.arrayContaining(['network_whitelist_add', 'network_whitelist_remove', 'network_forget']),
  )
  expect(backend.audit.find((e) => e.action === 'network_whitelist_add')?.network).toEqual({ network: '198.51.100.0/24', person: BOB })
})

test('an admin ends every session for a person', async ({ page, backend }) => {
  seed(backend)
  const panel = await openPanel(page)
  await panel.getByRole('button', { name: 'End every session for bob' }).click()
  await expect(panel.getByText('No active sessions.')).toBeVisible()
  await expect(panel.getByRole('button', { name: 'End every session for bob' })).toBeDisabled()
  expect(backend.http.find((h) => h.method === 'DELETE' && decodeURIComponent(h.path) === `/api/v1/users/${BOB}/sessions`)?.csrf).toBe('csrf-fake-1')
})

test('from an untrusted network whitelisting is refused, audited and then disabled', async ({ page, backend }) => {
  seed(backend)
  const panel = await openPanel(page)
  // The network stops being trusted mid-session; the page has not noticed yet.
  backend.me = sessions.untrusted({ role: 'admin', needs_answer: false })
  await panel.getByRole('textbox', { name: 'Network address' }).fill('198.51.100.7')
  await panel.getByRole('button', { name: 'Whitelist' }).click()
  await expect(panel.getByRole('alert')).toContainText('not from this network')
  expect(backend.audit.find((e) => e.action === 'network_refused')?.outcome).toEqual({ kind: 'denied', reason: 'untrusted_network' })
  expect(backend.userNetworks[BOB]?.map((n) => n.key)).toEqual(['203.0.113.7/32'])

  // After the refusal the page knows: the controls are disabled with the reason.
  await expect(page.getByTestId('restricted-banner')).toBeVisible()
  const wl = panel.getByRole('button', { name: 'Whitelist' })
  await expect(wl).toBeDisabled()
  await expect(wl).toHaveAttribute('data-blocked', 'untrusted_network')
  await expect(panel.getByRole('button', { name: 'Forget 203.0.113.7/32' })).toBeDisabled()

  // Ending sessions still works from here.
  await panel.getByRole('button', { name: 'End every session for bob' }).click()
  await expect(panel.getByText('No active sessions.')).toBeVisible()
})

/**
 * Spec 052 US2 (T017): Your sessions. Every session and known network is listed with the current
 * one marked; ending other sessions and changing an answer work from an untrusted network, and a
 * change to "home" never trusts the network at once.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

function seed(backend: FakeBackend): void {
  backend.sessionList = [
    f.sessionView('cur1', { current: true, last_used: '2026-10-23T11:59:00Z' }),
    f.sessionView('old2', { browser: 'Chrome on Android <b>bold</b>', address: '198.51.100.9', network: '198.51.100.9/32', trusted: false, location: 'Lyon, France' }),
    f.sessionView('old3', { browser: 'x'.repeat(120), created: '2026-10-20T08:00:00Z', last_used: '2026-10-22T08:00:00Z' }),
  ]
  backend.networks = [
    f.networkView('203.0.113.7/32', { source: 'first', trusted: true, reason: 'first', current: true, location: 'private network' }),
    f.networkView('198.51.100.9/32', { answer: 'public', source: 'answer', reason: 'public', location: 'Lyon, France' }),
  ]
}

const list = (page: Page) => page.getByRole('list', { name: 'Your sessions' })

test('the top bar leads to Your sessions, which lists every session plainly', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/')
  await page.getByRole('link', { name: 'Your sessions' }).click()
  await expect(page.getByRole('heading', { name: 'Your sessions', level: 1 })).toBeVisible()

  await expect(list(page).locator('[data-session]')).toHaveCount(3)
  await expect(list(page).locator('[data-session]').first()).toHaveAttribute('data-current', 'true')
  await expect(list(page).locator('[data-session="cur1"] [data-badge="current"]')).toHaveText('This session')

  const old2 = list(page).locator('[data-session="old2"]')
  // Plain text: the markup in the browser string is shown, not rendered.
  await expect(old2.locator('[data-field="browser"]')).toHaveText('Chrome on Android <b>bold</b>')
  await expect(old2.locator('b')).toHaveCount(0)
  await expect(old2.locator('[data-field="trust"]')).toHaveText('Restricted network')
  await expect(old2.locator('[data-field="location"]')).toHaveText('Lyon, France')
  const long = await list(page).locator('[data-session="old3"] [data-field="browser"]').textContent()
  expect(long?.length).toBe(48)
})

test('from an untrusted network, end one session and then all the others', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.untrusted({ needs_answer: false })
  await page.goto('/account/sessions')
  await expect(list(page).locator('[data-session]')).toHaveCount(3)

  await list(page).getByRole('button', { name: 'End session old3' }).click()
  await expect(list(page).locator('[data-session]')).toHaveCount(2)
  expect(backend.http.find((h) => h.method === 'DELETE' && h.path === '/api/v1/sessions/old3')?.csrf).toBe('csrf-fake-1')

  await page.getByRole('button', { name: 'End all other sessions' }).click()
  await expect(list(page).locator('[data-session]')).toHaveCount(1)
  await expect(page.getByRole('button', { name: 'End all other sessions' })).toBeDisabled()
  expect(backend.audit.filter((e) => e.action === 'session_end').map((e) => e.network?.session)).toEqual(['old3', 'others'])
  expect(backend.audit.filter((e) => e.action === 'network_refused')).toHaveLength(0)
})

test('changing an answer to home is recorded and does not trust the network', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/account/sessions')
  const nets = page.getByRole('list', { name: 'Your networks' })
  await expect(nets.locator('[data-network]').first()).toHaveAttribute('data-current', 'true')

  await nets.getByRole('combobox', { name: 'Answer for 198.51.100.9/32' }).selectOption('home')
  const net = nets.locator('[data-network="198.51.100.9/32"]')
  await expect(net).toHaveAttribute('data-reason', 'maturing')
  await expect(net.locator('[data-field="trust"]')).toHaveAttribute('data-trusted', 'false')
  await expect(page.getByRole('status')).toContainText('Saved: home')
  expect(backend.audit.find((e) => e.action === 'network_answer')?.network).toEqual({ network: '198.51.100.9/32', answer: 'home' })
})

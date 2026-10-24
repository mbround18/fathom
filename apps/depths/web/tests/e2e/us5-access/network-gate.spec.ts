/**
 * Spec 052 US1 (T013): on a network Depths has not seen for this person, the home/public question,
 * the restricted-view banner, and controls refused with the reason. Start, stop and restart still
 * work. The fake gates every command against `backend.me.network` as the server does, so a control
 * the page still offers is refused there too.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

const banner = (page: Page) => page.getByTestId('restricted-banner')
const question = (page: Page) => page.getByTestId('network-question')
const actions = (page: Page) => page.locator('[aria-label="Stack actions"]')

function seed(backend: FakeBackend): void {
  backend.stacks = f.stackList([
    f.stack('web', {
      has_env: true,
      services: [{ name: 'app', image: 'nginx:1.29', containers: [f.container({ name: 'web-app-1', project: 'web', service: 'app' })] }],
      running_services: 1,
      total_services: 1,
    }),
  ])
  backend.env.web = 'TOKEN=secret\n'
}

async function open(page: Page): Promise<void> {
  await page.goto('/stacks/web')
  await expect(page.getByRole('heading', { name: 'web', level: 1 })).toBeVisible()
}

/** The question is modal: the page behind it is hidden from the accessibility tree until it closes. */
async function openAsked(page: Page): Promise<void> {
  await page.goto('/stacks/web')
  await expect(question(page)).toBeVisible()
}

test('a trusted network shows no banner and asks nothing', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  await expect(actions(page).locator('[data-action="down"]')).toBeEnabled()
  await expect(banner(page)).toHaveCount(0)
  await expect(question(page)).toHaveCount(0)
})

test('an unknown network asks home or public; home starts the clock but stays restricted', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.untrusted()
  await openAsked(page)

  await expect(question(page)).toContainText('198.51.100.7/32')
  await expect(banner(page)).toHaveAttribute('data-reason', 'unknown')
  await question(page).getByRole('button', { name: 'Home' }).click()
  await expect(question(page)).toHaveCount(0)

  const answer = backend.http.find((h) => h.path === '/api/v1/networks/answer')
  expect(answer).toMatchObject({ method: 'POST', csrf: 'csrf-fake-1' })
  // The answer never trusts the network at once (FR-006): the banner names when it would.
  await expect(banner(page)).toHaveAttribute('data-reason', 'maturing')
  await expect(banner(page)).toContainText('trusted from 2026-10-30')
  expect(backend.audit.map((e) => e.action)).toContain('network_answer')

  // Refused controls carry the reason; start, stop and restart do not.
  for (const kind of ['down', 'pull', 'update', 'delete']) {
    const b = page.locator(`[data-action="${kind}"]`).first()
    await expect(b).toBeDisabled()
    await expect(b).toHaveAttribute('data-blocked', 'untrusted_network')
    await expect(b).toHaveAttribute('title', /Not from this network/)
  }
  for (const kind of ['up', 'stop', 'restart']) await expect(actions(page).locator(`[data-action="${kind}"]`)).toBeEnabled()
  await expect(page.locator('[data-open="shell"]').first()).toBeDisabled()
  await expect(page.locator('[data-action="reveal-env"]')).toBeDisabled()
})

test('public keeps the view restricted, and a restart still runs end to end', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.untrusted()
  await openAsked(page)
  await question(page).getByRole('button', { name: 'Public' }).click()
  await expect(banner(page)).toHaveAttribute('data-reason', 'public')
  await expect(banner(page)).toContainText('stays restricted')

  // SC-002: a restart from the restricted view, preview to finish.
  await actions(page).locator('[data-action="restart"]').click()
  await page.getByRole('dialog').locator('[data-confirm]').click()
  await expect(page.locator('[data-operation="restart"]')).toBeVisible()
  expect(backend.received.map((c) => c.type)).toContain('Confirm')
  expect(backend.audit.filter((e) => e.action === 'network_refused')).toHaveLength(0)
})

test('not now waits for the next load; the banner offers the question again', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.untrusted()
  await openAsked(page)
  await question(page).getByRole('button', { name: 'Not now' }).click()
  await expect(question(page)).toHaveCount(0)
  expect(backend.http.filter((h) => h.path === '/api/v1/networks/answer')).toHaveLength(0)
  await banner(page).getByRole('button', { name: 'Home or public?' }).click()
  await expect(question(page)).toBeVisible()
})

test('a network that turns untrusted mid-session is refused by the server, and audited', async ({ page, backend }) => {
  seed(backend)
  await open(page)
  // The page still thinks the network is trusted; the next command arrives from another one.
  backend.me = sessions.untrusted()
  await actions(page).locator('[data-action="down"]').click()
  await expect(page.getByRole('dialog').locator('[data-refusal="denied"]')).toContainText('Not from this network')
  const refused = backend.audit.filter((e) => e.action === 'network_refused')
  expect(refused.map((e) => e.network?.cmd)).toEqual(['PreviewOp'])
  expect(backend.received.filter((c) => c.type === 'Confirm')).toHaveLength(0)
})

test('an admin sees the proxy notice when addresses look hidden', async ({ page, backend }) => {
  seed(backend)
  backend.me = sessions.admin({ proxy_warning: true })
  await open(page)
  await expect(page.getByTestId('proxy-warning')).toContainText('DEPTHS_TRUSTED_PROXIES')
  await expect(banner(page)).toHaveCount(0)
})

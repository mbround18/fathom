/**
 * Passkey enrolment and step-up (spec 055 T030), with Chromium's virtual authenticator standing in
 * for the person's device, so `navigator.credentials` runs for real.
 */
import type { Page } from '@playwright/test'

import { expect, fixtures, test } from '../../support/fake-backend'

async function virtualAuthenticator(page: Page): Promise<void> {
  const cdp = await page.context().newCDPSession(page)
  await cdp.send('WebAuthn.enable')
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  })
}

async function accountMenu(page: Page, item: string): Promise<void> {
  await page.getByRole('button', { name: 'Account' }).click()
  await page.getByRole('menuitem', { name: item }).click()
}

test('registers a passkey, then verifies with it', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  await page.goto('/runs')

  await accountMenu(page, 'Register a passkey')
  const enrol = page.getByRole('dialog', { name: 'Register a passkey' })
  await enrol.getByLabel('Name').fill('Work laptop')
  await enrol.getByRole('button', { name: 'Register' }).click()
  await expect(page.getByRole('dialog', { name: 'Passkey registered' })).toBeVisible()
  expect(backend.passkeys.map((p) => p.name)).toEqual(['Work laptop'])
  const finish = await backend.waitForCmd('PasskeyFinish')
  expect(finish.data.credential.response).toHaveProperty('attestationObject')
  await page.getByRole('button', { name: 'Done' }).click()

  await accountMenu(page, 'Verify it’s you')
  await page.getByRole('dialog', { name: 'Verify it’s you' }).getByRole('button', { name: 'Use passkey' }).click()
  await expect(page.getByRole('dialog', { name: 'Verified' })).toBeVisible()
  expect(backend.me?.verification.verified).toBe(true)
})

test('shows the server refusing a passkey', async ({ page, backend }) => {
  await virtualAuthenticator(page)
  backend.on('PasskeyFinish', () => [{ type: 'PasskeyRefused', data: { reason: 'replayed' } }])
  await page.goto('/runs')
  await accountMenu(page, 'Register a passkey')
  const enrol = page.getByRole('dialog', { name: 'Register a passkey' })
  await enrol.getByLabel('Name').fill('Work laptop')
  await enrol.getByRole('button', { name: 'Register' }).click()
  await expect(enrol.getByRole('alert')).toHaveText('That passkey answer was already used. Try again.')
  await expect(enrol.getByRole('button', { name: 'Register' })).toBeEnabled()
})

test('a required enrolment shows the banner and opens on a push', async ({ page, backend }) => {
  backend.me = fixtures.me({ verification: fixtures.verification({ required: true }) })
  await page.goto('/runs')
  await expect(page.locator('[data-notice="enrol"]')).toBeVisible()
  await backend.waitForCmd('GetMe')
  backend.push({ type: 'PasskeyEnrolmentRequired' })
  const enrol = page.getByRole('dialog', { name: 'Register a passkey' })
  await expect(enrol).toContainText('needs a passkey')
  await enrol.getByRole('button', { name: 'Later' }).click()
  await expect(enrol).toBeHidden()
})

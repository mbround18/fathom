/**
 * 043 US2 (T020): sign-in is an optional step of the setup screen (after a pick when an identity
 * provider exists, or when the picked context answers 401), and Settings › Sign-in shows the same
 * controls. The commands match the identity chip dialog's (033).
 */
import { expect, test, type Page } from '@playwright/test'

import type { OidcIdentity } from '../../src/protocol/types'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, navigateTo, open, sent } from './helpers'

const issuer = {
  id: 'rauthy',
  issuer_url: 'https://auth.example.com/auth/v1',
  client_id: 'fathom',
  scopes: ['openid'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

const identity = {
  issuer_id: 'rauthy',
  sub: 'u1',
  username: 'ada',
  email: null,
  groups: [],
  expires_at: 4102444800,
  state: { type: 'SignedIn' },
} satisfies OidcIdentity

const setup = (page: Page) => page.getByTestId('setup-screen')
const step = (page: Page) => page.getByTestId('setup-sign-in-step')

async function withIssuer(page: Page) {
  await emit(
    page,
    batch([
      system(10, {
        type: 'OidcConfigLoaded',
        data: { config: { issuers: [issuer], contexts: [], role_map: {} }, identities: [] },
      } as never),
    ]),
  )
}

async function pick(page: Page, context: string) {
  await setup(page).getByRole('option', { name: context }).click()
}

test.describe('043 sign-in in setup', () => {
  test('offered after a pick when an issuer exists; Skip continues with the kubeconfig', async ({
    page,
  }) => {
    await open(page, 'no-current-context')
    await withIssuer(page)
    await pick(page, 'kind-b')
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'sign-in')
    await expect(page.getByRole('heading', { name: 'Sign in (optional)' })).toBeVisible()
    await expect(step(page).getByTestId('oidc-issuer-rauthy')).toBeVisible()
    // Only the picked context's authentication is offered.
    await expect(step(page).getByTestId('oidc-context-mode-kind-b')).toBeVisible()
    await expect(step(page).getByTestId('oidc-context-mode-kind-a')).toHaveCount(0)
    await step(page).getByRole('button', { name: 'Skip' }).click()
    await expect(setup(page)).toHaveCount(0)
  })

  test('a device-code sign-in shows its progress, then Continue', async ({ page }) => {
    await open(page, 'no-current-context')
    await withIssuer(page)
    await pick(page, 'kind-b')
    await clearSent(page)
    await step(page).getByTestId('oidc-signin-device-rauthy').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'OidcSignIn'))
      .toMatchObject({ data: { issuer_id: 'rauthy', method: 'DeviceCode' } })
    await emit(
      page,
      batch([
        system(13, {
          type: 'OidcSignInProgress',
          data: {
            issuer_id: 'rauthy',
            stage: {
              type: 'DeviceCode',
              data: {
                user_code: 'ABCD-1234',
                verification_uri: 'https://auth/device',
                expires_at: 1,
              },
            },
          },
        }),
      ]),
    )
    await expect(step(page).getByTestId('oidc-device-code-rauthy')).toHaveText('ABCD-1234')
    await emit(
      page,
      batch([system(14, { type: 'OidcIdentityUpdated', data: { issuer_id: 'rauthy', identity } })]),
    )
    await expect(step(page).getByTestId('oidc-identity-rauthy')).toContainText('Signed in as ada')
    await step(page).getByTestId('setup-sign-in-continue').click()
    await expect(setup(page)).toHaveCount(0)
  })

  test('binding the context to the issuer sends the same command as the identity dialog', async ({
    page,
  }) => {
    await open(page, 'no-current-context')
    await withIssuer(page)
    await pick(page, 'kind-b')
    await clearSent(page)
    await step(page).getByTestId('oidc-context-mode-kind-b').selectOption('rauthy')
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SetContextAuthMode'))
      .toMatchObject({ data: { context: 'kind-b', mode: 'oidc', issuer_id: 'rauthy' } })
  })

  test('a 401 offers the step with no issuer: "Add an identity provider" and Skip', async ({
    page,
  }) => {
    await open(page, 'setup-unauthorized')
    await pick(page, 'kind-a')
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'sign-in')
    await expect(step(page)).toContainText('turned down')
    await expect(step(page).getByTestId('sign-in-no-issuer')).toBeVisible()
    await step(page).getByRole('button', { name: 'Add an identity provider' }).click()
    await expect(step(page).getByTestId('oidc-issuer-settings')).toBeVisible()
    await step(page).getByRole('button', { name: 'Skip' }).click()
    // Skipped: the failure is explained, and not re-offered until the next pick.
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'failed')
    await expect(setup(page)).toContainText('kind-a rejected the credentials')
    await setup(page).getByTestId('setup-sign-in').click()
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'sign-in')
  })

  test('Settings › Sign-in renders the same controls', async ({ page }) => {
    await open(page)
    await withIssuer(page)
    await navigateTo(page, 'Sign-in', 'Settings')
    const settings = page.getByTestId('sign-in-settings')
    await expect(settings).toBeVisible()
    await expect(settings.getByTestId('oidc-issuer-rauthy')).toBeVisible()
    await expect(settings.getByTestId('oidc-signin-device-rauthy')).toBeVisible()
    await expect(settings.getByTestId('oidc-context-mode-kind-a')).toBeVisible()
    await expect(settings.getByTestId('oidc-context-mode-kind-b')).toBeVisible()
    await clearSent(page)
    await settings.getByTestId('oidc-context-mode-kind-a').selectOption('rauthy')
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SetContextAuthMode'))
      .toMatchObject({ data: { context: 'kind-a', mode: 'oidc', issuer_id: 'rauthy' } })
  })
})

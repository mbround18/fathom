/**
 * 033 US1/US2: OIDC identity chip, sign-in, cancel, expiry and sign-out against the fake backend.
 */
import { expect, test } from '@playwright/test'

import type { OidcIdentity } from '../../src/protocol/types'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, open, sent } from './helpers'

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

test.describe('033 identity', () => {
  test('sign in, cancel, expire and sign out', async ({ page }) => {
    await open(page)
    await emit(
      page,
      batch([
        system(10, {
          type: 'OidcConfigLoaded',
          data: {
            config: { issuers: [issuer], contexts: [], role_map: {} },
            identities: [],
          },
        } as never),
      ]),
    )
    await page.getByTestId('identity-chip').click()
    await expect(page.getByTestId('oidc-issuer-rauthy')).toBeVisible()

    await clearSent(page)
    await page.getByTestId('oidc-signin-browser-rauthy').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'OidcSignIn'))
      .toMatchObject({ data: { issuer_id: 'rauthy', method: 'Browser' } })

    await emit(
      page,
      batch([
        system(11, {
          type: 'OidcSignInProgress',
          data: { issuer_id: 'rauthy', stage: { type: 'Awaiting' } },
        }),
      ]),
    )
    await expect(page.getByTestId('oidc-awaiting-rauthy')).toBeVisible()
    await page.getByTestId('oidc-cancel-rauthy').click()
    await emit(
      page,
      batch([
        system(12, {
          type: 'OidcError',
          data: { issuer_id: 'rauthy', stage: 'cancelled', message: 'sign-in cancelled' },
        }),
      ]),
    )
    await expect(page.getByTestId('oidc-error-rauthy')).toContainText('sign-in cancelled')

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
    await expect(page.getByTestId('oidc-device-code-rauthy')).toHaveText('ABCD-1234')

    await emit(
      page,
      batch([
        system(14, {
          type: 'OidcIdentityUpdated',
          data: { issuer_id: 'rauthy', identity },
        }),
      ]),
    )
    await expect(page.getByTestId('oidc-identity-rauthy')).toContainText('Signed in as ada')

    await emit(
      page,
      batch([
        system(15, {
          type: 'OidcSessionEnded',
          data: { issuer_id: 'rauthy', reason: 'refresh token expired' },
        }),
      ]),
    )
    await expect(page.getByTestId('oidc-identity-rauthy')).toContainText('Signed out')

    await emit(
      page,
      batch([
        system(16, {
          type: 'OidcIdentityUpdated',
          data: { issuer_id: 'rauthy', identity },
        }),
      ]),
    )
    await clearSent(page)
    await page.getByTestId('oidc-signout-rauthy').click()
    await expect.poll(async () => (await sent(page)).map((c) => c.type)).toContain('OidcSignOut')
  })

  test('per-context opt-in sends SetContextAuthMode', async ({ page }) => {
    await open(page)
    await emit(
      page,
      batch([
        system(10, {
          type: 'OidcConfigLoaded',
          data: { config: { issuers: [issuer], contexts: [], role_map: {} }, identities: [] },
        } as never),
      ]),
    )
    await page.getByTestId('identity-chip').click()
    const select = page.locator('[data-testid^="oidc-context-mode-"]').first()
    await select.selectOption('rauthy')
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SetContextAuthMode'))
      .toMatchObject({ data: { mode: 'oidc', issuer_id: 'rauthy' } })
  })
})

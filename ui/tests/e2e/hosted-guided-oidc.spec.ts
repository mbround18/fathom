/**
 * 045 hosted (T073): the guided path in hosted mode, where the issuer is the operator's and the
 * cluster's authentication is read-only.
 *
 * Hosted is the mirror image of the desktop path: the issuer arrives from the environment rather
 * than being typed (FR-031), it is shown but not edited (FR-032/FR-033), the API server read is
 * the one guided step hosted keeps (FR-023), and the surface appears at all only for a user the
 * API server would let read the seed (FR-034/FR-035). All of that reaches the UI as events the
 * server emits, so the fake transport can script it exactly as `HostedTenantHost` sends it:
 * `OidcConfigLoaded` + `OidcSeedOrigin` for a permitted user, a warning for a refused one.
 *
 * Run (fake transport, like every other spec in this directory):
 *   pnpm -C ui e2e -- hosted-guided-oidc
 * The live hosted suite is `ui/tests/hosted/` (`pnpm -C ui e2e:hosted`) and needs a kind cluster;
 * nothing here does.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, read, system } from '../fixtures/builders'
import { emit, navigateTo, openHosted, sent, settleFakeReplies } from './helpers'

const ISSUER_URL = 'https://auth.localtest.me/auth/v1/'

/** Exactly what `oidc_seed::seeded_issuer` projects: no `client_secret` anywhere in it. */
const SEEDED_ISSUER = {
  id: 'hosted',
  issuer_url: ISSUER_URL,
  client_id: 'fathom',
  scopes: ['openid', 'profile', 'email', 'groups'],
  ca_bundle_path: '/etc/fathom/ca/ca.crt',
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

/** `SeedObjectRef::source()`, verbatim: the place the operator changes the seed (FR-032). */
const SEED_SOURCE = 'FATHOM_OIDC_* (mounted Secret fathom/fathom-oidc)'

/** The two events the hosted server emits on attach once the access review says yes. */
function seedEvents() {
  return batch([
    system(10, {
      type: 'OidcConfigLoaded',
      data: {
        config: { issuers: [SEEDED_ISSUER], contexts: [], role_map: {} },
        identities: [],
      },
    } as never),
    system(11, {
      type: 'OidcSeedOrigin',
      data: { issuer_id: 'hosted', origin: 'SeededFromEnvironment', source: SEED_SOURCE },
    } as never),
  ])
}

/**
 * Sign in as a hosted admin and open the seeded issuer. The identity chip is hidden in hosted,
 * so the path is reached through Settings › Sign-in, which is the route T072 keeps open.
 */
async function openSeededPath(page: Page): Promise<void> {
  await openHosted(page, 'user=ada&groups=fathom-admins')
  await emit(page, seedEvents())
  await navigateTo(page, 'Sign-in', 'Settings')
  await expect(page.getByTestId('oidc-issuer-hosted')).toBeVisible()
  await page.getByTestId('oidc-review-hosted').click()
  await expect(page.getByTestId('guided-issuer-path')).toBeVisible()
}

test.describe('045 hosted: the seeded issuer, shown and not edited', () => {
  test('the issuer list states the origin and offers no edit or delete', async ({ page }) => {
    await openHosted(page, 'user=ada&groups=fathom-admins')
    await emit(page, seedEvents())
    await navigateTo(page, 'Sign-in', 'Settings')

    const row = page.getByTestId('oidc-issuer-hosted')
    await expect(row).toContainText(ISSUER_URL)
    // FR-032: the origin is stated, and so is the object it is changed in.
    const origin = page.getByTestId('oidc-issuer-origin-hosted')
    await expect(origin).toContainText('seeded')
    await expect(origin).toContainText(SEED_SOURCE)
    // FR-033: nothing in this screen can change or remove it -- the Secret can.
    await expect(page.getByTestId('oidc-edit-hosted')).toHaveCount(0)
    await expect(page.getByTestId('oidc-delete-hosted')).toHaveCount(0)
    await expect(page.getByTestId('oidc-review-hosted')).toBeVisible()
  })

  test('the path opens read-only with no save step and writes nothing', async ({ page }) => {
    await openSeededPath(page)

    // The fields are shown with the seeded values, and none of them is editable (FR-032).
    await expect(page.getByTestId('guided-issuer-url')).toHaveValue(ISSUER_URL)
    await expect(page.getByTestId('guided-issuer-url')).not.toBeEditable()
    await expect(page.getByTestId('guided-client-id')).toHaveValue('fathom')
    await expect(page.getByTestId('guided-client-id')).not.toBeEditable()
    await expect(page.getByTestId('guided-seeded-origin')).toContainText(SEED_SOURCE)

    // `stepsOf` drops `review-save` for a seeded draft: there is nothing here to save, so the
    // save-shaped steps never appear and no write command can be sent from this path.
    await expect(page.getByTestId('guided-step-review-save')).toHaveCount(0)
    await expect(page.getByTestId('guided-step-apiserver-write')).toHaveCount(0)
    await settleFakeReplies(page)
    const types = (await sent(page)).map((c) => c.type)
    expect(types).not.toContain('OidcSaveDraft')
    expect(types).not.toContain('OidcSaveIssuer')
    expect(types).not.toContain('WriteApiServerAuth')
  })

  test('steps 1-6 are all reachable: discovery, claims and the comparison stay available', async ({
    page,
  }) => {
    await openSeededPath(page)
    for (const step of [
      'issuer-url',
      'discovery',
      'test-sign-in',
      'claims',
      'cluster-compare',
      'fathom-roles',
    ]) {
      await expect(page.getByTestId(`guided-step-${step}`)).toBeEnabled()
      await page.getByTestId(`guided-step-${step}`).click()
      await expect(page.getByTestId(`guided-step-body-${step}`)).toBeVisible()
    }
  })
})

test.describe('045 hosted: what this cluster accepts is read, never written', () => {
  const comparison = {
    read: {
      style: 'StructuredAuthn',
      issuer: ISSUER_URL,
      client_id: 'fathom',
      username_claim: 'preferred_username',
      groups_claim: 'groups',
      username_prefix: 'oidc:',
      groups_prefix: 'oidc:',
      context: 'in-cluster',
      checked: true,
      reason: null,
    },
    differences: [],
    // FR-023/FR-031: hosted reads how the cluster authenticates and stops there. The refusal is
    // the server's, stated as a reason rather than as a disabled button with no explanation.
    writable: false,
    not_writable_reason:
      'Fathom hosted does not change how the cluster authenticates: WriteApiServerAuth is not \
available in hosted mode. Apply the change with your cluster tooling.',
  }

  test('the comparison runs and the write is refused with the reason given', async ({ page }) => {
    await openSeededPath(page)
    await page.getByTestId('guided-step-cluster-compare').click()
    await expect
      .poll(async () => (await sent(page)).some((c) => c.type === 'ReadApiServerAuth'))
      .toBe(true)
    await emit(
      page,
      batch([
        read(40, clusterRoute('in-cluster'), {
          type: 'ApiServerAuthRead',
          data: comparison,
        } as never),
      ]),
    )

    await expect(page.getByTestId('guided-compare-context')).toContainText('in-cluster')
    await expect(page.getByTestId('guided-compare-agrees')).toBeVisible()
    await expect(page.getByTestId('guided-compare-issuer')).toHaveAttribute('data-agrees', 'true')
    const refused = page.getByTestId('guided-compare-not-writable')
    await expect(refused).toContainText('not available in hosted mode')
    await expect(refused).toContainText('your cluster tooling')
    expect((await sent(page)).map((c) => c.type)).not.toContain('WriteApiServerAuth')
  })
})

test.describe('045 hosted: a user the API server would refuse (FR-034/FR-035)', () => {
  /** `oidc_seed::denied_notice` for a verified denial, verbatim. */
  const DENIED =
    "Fathom's OIDC configuration is not shown: the API server refused `get secrets fathom/fathom-oidc` — " +
    'secrets "fathom-oidc" is forbidden: User "carol" cannot get resource "secrets" in API group "" in the namespace "fathom"'

  test('the refusal is the API servers own words, and no issuer is shown', async ({ page }) => {
    await openHosted(page, 'user=carol&groups=')
    // No seed events: a refused review emits the warning *instead of* the snapshot, so the UI
    // never holds a seeded issuer it would have to hide again.
    await emit(
      page,
      batch([system(10, { type: 'Notification', data: { level: 'Warn', message: DENIED } })]),
    )

    const notice = page.getByTestId('status-notice')
    await expect(notice).toBeVisible()
    await page.getByTestId('status-notice-details').click()
    const detail = page.getByTestId('status-notice-popover')
    await expect(detail).toContainText('cannot get resource "secrets"')
    await expect(detail).toContainText('get secrets fathom/fathom-oidc')
    await page.keyboard.press('Escape')

    await navigateTo(page, 'Sign-in', 'Settings')
    await expect(page.getByTestId('oidc-no-issuers')).toBeVisible()
    await expect(page.getByTestId('oidc-issuer-hosted')).toHaveCount(0)
  })

  test('an unverifiable review is stated as an unknown and still refuses', async ({ page }) => {
    await openHosted(page, 'user=carol&groups=')
    const unknown =
      "Fathom's OIDC configuration is not shown: `get secrets fathom/fathom-oidc` could not be checked, " +
      'so it is treated as not permitted — error sending request for url: connection refused'
    await emit(
      page,
      batch([system(10, { type: 'Notification', data: { level: 'Warn', message: unknown } })]),
    )
    await page.getByTestId('status-notice-details').click()
    const detail = page.getByTestId('status-notice-popover')
    // FR-035: the unknown is said to be an unknown, and the verbatim cause is kept.
    await expect(detail).toContainText('could not be checked, so it is treated as not permitted')
    await expect(detail).toContainText('connection refused')
  })
})

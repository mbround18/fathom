/**
 * 045 US1-US3: the guided OIDC path, against the fake transport.
 *
 * Every reply is scripted here rather than canned in a scenario, because what these tests assert
 * is exactly the difference between two replies: a discovery document that agrees with the URL
 * and one that does not, an API server that matches the draft and one that disagrees on a prefix.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, clusterRoute, read, system } from '../fixtures/builders'
import { clearSent, emit, open, sent, settleFakeReplies } from './helpers'

const ISSUER_URL = 'https://idp.example.com/realms/fathom'

/** A discovery document as a provider writes it; `issuer` is what the UI compares the URL against. */
function discovery(issuer = ISSUER_URL) {
  return {
    issuer,
    authorization_endpoint: `${ISSUER_URL}/protocol/openid-connect/auth`,
    token_endpoint: `${ISSUER_URL}/protocol/openid-connect/token`,
    device_authorization_endpoint: `${ISSUER_URL}/protocol/openid-connect/auth/device`,
    jwks_uri: `${ISSUER_URL}/protocol/openid-connect/certs`,
    end_session_endpoint: null,
    scopes_supported: ['openid', 'profile', 'email', 'groups'],
    claims_supported: ['sub', 'preferred_username', 'groups', 'email'],
  }
}

const SAVED_ISSUER = {
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

/** The path opens from the identity dialog; mounting it elsewhere is T053's job, not these tests'. */
async function openPath(page: Page): Promise<void> {
  await open(page)
  await emit(
    page,
    batch([
      system(10, {
        type: 'OidcConfigLoaded',
        data: { config: { issuers: [SAVED_ISSUER], contexts: [], role_map: {} }, identities: [] },
      } as never),
    ]),
  )
  await page.getByTestId('identity-chip').click()
  await page.getByTestId('oidc-guided-add').click()
  await expect(page.getByTestId('guided-issuer-path')).toBeVisible()
}

/** Type the URL and let the blur fire the probe, as US1 #1 describes it. */
async function probe(page: Page, url = ISSUER_URL): Promise<void> {
  await page.getByTestId('guided-issuer-url').fill(url)
  await page.getByTestId('guided-client-id').fill('fathom')
  await page.getByTestId('guided-issuer-id').fill('idp')
  await expect
    .poll(async () => (await sent(page)).find((c) => c.type === 'OidcDiscoverDraft'))
    .toEqual({ type: 'OidcDiscoverDraft', data: { issuer_url: url, ca_bundle_path: null } })
}

async function deliverDiscovery(page: Page, issuer = ISSUER_URL): Promise<void> {
  const requestId = await page.evaluate(
    () => window.__fathomFake?.requestIdOf('OidcDiscoverDraft') ?? 0,
  )
  await emit(
    page,
    batch([
      system(20, {
        type: 'OidcDraftDiscovered',
        data: { issuer_url: ISSUER_URL, discovery: discovery(issuer) },
      } as never),
    ]),
  )
  await page.evaluate((id) => window.__fathomFake?.finish(id, true), requestId)
  await settleFakeReplies(page)
}

test.describe('045 US1: the discovery document, as the provider wrote it', () => {
  test('probes on leaving the URL field and renders the real document', async ({ page }) => {
    await openPath(page)
    await probe(page)
    await deliverDiscovery(page)

    // Arriving at the issuer-url step, the document advances the path to where it is read.
    await expect(page.getByTestId('guided-step-body-discovery')).toBeVisible()
    await expect(page.getByTestId('guided-declared-issuer')).toHaveText(ISSUER_URL)
    await expect(page.getByTestId('guided-token-endpoint')).toHaveText(
      `${ISSUER_URL}/protocol/openid-connect/token`,
    )
    await expect(page.getByTestId('guided-jwks-uri')).toHaveText(
      `${ISSUER_URL}/protocol/openid-connect/certs`,
    )
    // Every advertised scope and claim, listed rather than summarised (FR-001).
    for (const scope of ['openid', 'profile', 'email', 'groups']) {
      await expect(page.getByTestId(`guided-scopes-supported-${scope}`)).toBeVisible()
    }
    for (const claim of ['sub', 'preferred_username', 'groups', 'email']) {
      await expect(page.getByTestId(`guided-claims-supported-${claim}`)).toBeVisible()
    }
    await expect(page.getByTestId('guided-device-code-support')).toContainText('device-code')
    await expect(page.getByTestId('guided-next')).toBeEnabled()
  })

  test('a declared issuer that disagrees with the URL blocks progress, showing both', async ({
    page,
  }) => {
    await openPath(page)
    await probe(page)
    await deliverDiscovery(page, 'https://idp.example.com/realms/other')

    const refusal = page.getByTestId('guided-issuer-mismatch')
    await expect(refusal).toBeVisible()
    await expect(page.getByTestId('guided-mismatch-typed')).toHaveText(ISSUER_URL)
    await expect(page.getByTestId('guided-mismatch-declared')).toHaveText(
      'https://idp.example.com/realms/other',
    )
    // The one refusal in the path: forward is shut, and so is the step nav past it (FR-002).
    await expect(page.getByTestId('guided-next')).toBeDisabled()
    await expect(page.getByTestId('guided-step-claims')).toBeDisabled()
  })

  test('a failed probe shows the provider words verbatim and retries on the same step', async ({
    page,
  }) => {
    await openPath(page)
    await probe(page)
    const requestId = await page.evaluate(
      () => window.__fathomFake?.requestIdOf('OidcDiscoverDraft') ?? 0,
    )
    await page.evaluate(
      (id) =>
        window.__fathomFake?.finish(
          id,
          false,
          'error sending request for url: self signed certificate in certificate chain',
        ),
      requestId,
    )
    const error = page.getByTestId('guided-probe-error')
    await expect(error).toContainText('self signed certificate in certificate chain')
    await expect(error).toContainText(ISSUER_URL)

    // FR-003: supply the CA bundle and retry here, without losing the rest of the draft.
    await clearSent(page)
    await page.getByTestId('guided-ca-bundle').fill('/etc/ssl/corp.pem')
    await page.getByTestId('guided-probe').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'OidcDiscoverDraft'))
      .toEqual({
        type: 'OidcDiscoverDraft',
        data: { issuer_url: ISSUER_URL, ca_bundle_path: '/etc/ssl/corp.pem' },
      })
    await expect(page.getByTestId('guided-client-id')).toHaveValue('fathom')
  })
})

test.describe('045 US2: claims from a real token, with the prefixes applied', () => {
  test('a scripted sign-in fills the claims table and the prefixed preview', async ({ page }) => {
    await openPath(page)
    await probe(page)
    await deliverDiscovery(page)

    await page.getByTestId('guided-step-test-sign-in').click()
    await clearSent(page)
    await page.getByTestId('guided-test-sign-in').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'OidcTestSignIn'))
      .toMatchObject({
        type: 'OidcTestSignIn',
        data: {
          draft: { config: { issuer_url: ISSUER_URL, client_id: 'fathom' }, method: 'Browser' },
        },
      })

    await emit(
      page,
      batch([
        system(30, {
          type: 'OidcDraftSignInSucceeded',
          data: {
            result: {
              username: 'ada',
              groups: ['platform', 'oncall'],
              claims: [
                { name: 'sub', value: 'u-1' },
                { name: 'preferred_username', value: 'ada' },
                { name: 'groups', value: 'platform, oncall' },
              ],
              expires_at: 1790000000,
            },
          },
        } as never),
      ]),
    )
    await expect(page.getByTestId('guided-sign-in-username')).toHaveText('ada')

    await page.getByTestId('guided-step-claims').click()
    await expect(page.getByTestId('guided-claims-observed')).toBeVisible()
    await expect(page.getByTestId('guided-claims-table')).toBeVisible()
    await expect(page.getByTestId('guided-claim-preferred_username')).toContainText('ada')
    await expect(page.getByTestId('guided-claim-sub')).toContainText('u-1')
    // FR-005: the claim is chosen from the claims the token carried, not typed from memory.
    await expect(page.getByTestId('guided-username-claim')).toHaveValue('preferred_username')

    // FR-006/FR-011: editing a prefix moves the preview with it, before anything is saved.
    await expect(page.getByTestId('guided-preview-username')).toHaveText('oidc:ada')
    await expect(page.getByTestId('guided-preview-groups')).toHaveText('oidc:platform, oidc:oncall')
    await page.getByTestId('guided-groups-prefix').fill('corp:')
    await expect(page.getByTestId('guided-preview-groups')).toHaveText('corp:platform, corp:oncall')
  })

  test('with no sign-in the claims are the advertised ones, and said to be', async ({ page }) => {
    await openPath(page)
    await probe(page)
    await deliverDiscovery(page)

    await page.getByTestId('guided-step-claims').click()
    const label = page.getByTestId('guided-claims-advertised')
    await expect(label).toContainText('advertises')
    await expect(label).toContainText('not what a token was observed to carry')
    await expect(page.getByTestId('guided-claims-table')).toHaveCount(0)
    await expect(page.getByTestId('guided-identity-preview-pattern')).toBeVisible()
  })

  test('closing the path writes nothing and leaves the issuer list alone', async ({ page }) => {
    await openPath(page)
    await probe(page)
    await deliverDiscovery(page)
    await page.getByTestId('guided-step-claims').click()
    await page.getByTestId('guided-username-prefix').fill('changed:')

    await clearSent(page)
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('guided-issuer-path')).toHaveCount(0)
    // FR-007: nothing persisted, by discarding the draft rather than by cleaning up after it.
    await settleFakeReplies(page)
    const types = (await sent(page)).map((c) => c.type)
    expect(types).not.toContain('OidcSaveDraft')
    expect(types).not.toContain('OidcSaveIssuer')
    await expect(page.getByTestId('oidc-issuer-rauthy')).toBeVisible()
    await expect(page.getByTestId('oidc-issuer-rauthy')).toContainText(SAVED_ISSUER.issuer_url)
  })
})

test.describe('045 US3: what this cluster accepts', () => {
  /** Walk to the comparison step and script the API server's reply. */
  async function compareWith(page: Page, comparison: object, context = 'kind-a'): Promise<void> {
    await openPath(page)
    await probe(page)
    await deliverDiscovery(page)
    await clearSent(page)
    await page.getByTestId('guided-step-cluster-compare').click()
    await expect
      .poll(async () => (await sent(page)).some((c) => c.type === 'ReadApiServerAuth'))
      .toBe(true)
    await emit(
      page,
      batch([
        read(40, clusterRoute(context), { type: 'ApiServerAuthRead', data: comparison } as never),
      ]),
    )
  }

  const matching = {
    read: {
      style: 'Legacy',
      issuer: ISSUER_URL,
      client_id: 'fathom',
      username_claim: 'preferred_username',
      groups_claim: 'groups',
      username_prefix: 'oidc:',
      groups_prefix: 'oidc:',
      context: 'kind-a',
      checked: true,
      reason: null,
    },
    differences: [],
    writable: true,
    not_writable_reason: null,
  }

  test('a clean match names the context and says every field agrees', async ({ page }) => {
    await compareWith(page, matching)
    await expect(page.getByTestId('guided-compare-context')).toContainText('kind-a')
    await expect(page.getByTestId('guided-compare-agrees')).toBeVisible()
    await expect(page.getByTestId('guided-compare-issuer')).toHaveAttribute('data-agrees', 'true')
    await expect(page.getByTestId('guided-compare-style')).toHaveAttribute('data-style', 'Legacy')
    // Read-only, and reached before any save: no write command may have been sent.
    const types = (await sent(page)).map((c) => c.type)
    expect(types).not.toContain('WriteApiServerAuth')
    expect(types).not.toContain('OidcSaveDraft')
  })

  test('a prefix mismatch is reported before any save, as a binding problem', async ({ page }) => {
    await compareWith(page, {
      ...matching,
      read: { ...matching.read, groups_prefix: 'k8s:' },
      differences: [{ field: 'groups_prefix', cluster: 'k8s:', draft: 'oidc:' }],
    })
    const row = page.getByTestId('guided-compare-groups_prefix')
    await expect(row).toHaveAttribute('data-agrees', 'false')
    await expect(row).toContainText('k8s:')
    await expect(row).toContainText('oidc:')
    await expect(page.getByTestId('guided-compare-issuer')).toHaveAttribute('data-agrees', 'true')
    // FR-009: tokens are accepted, the bindings are what will not match. Different problem.
    const consequence = page.getByTestId('guided-compare-consequence-groups_prefix')
    await expect(consequence).toContainText('Tokens are accepted')
    await expect(consequence).toContainText('bindings')
    await expect(page.getByTestId('guided-compare-agrees')).toHaveCount(0)
    // A mismatch is information, not a refusal: the path still moves on (FR-008).
    await expect(page.getByTestId('guided-next')).toBeEnabled()
    expect((await sent(page)).map((c) => c.type)).not.toContain('OidcSaveDraft')
  })

  test('an unreadable API server is shown verbatim and does not block', async ({ page }) => {
    await compareWith(page, {
      read: {
        style: 'Unknown',
        issuer: null,
        client_id: null,
        username_claim: null,
        groups_claim: null,
        username_prefix: null,
        groups_prefix: null,
        context: 'kind-a',
        checked: false,
        reason:
          'pods "kube-apiserver-fathom-dev-control-plane" is forbidden: User "ada" cannot get resource "pods" in API group "" in the namespace "kube-system"',
      },
      differences: [],
      writable: false,
      not_writable_reason: 'This control plane was not provisioned by Fathom.',
    })
    const unknown = page.getByTestId('guided-compare-unknown')
    await expect(unknown).toContainText('is forbidden: User "ada" cannot get resource "pods"')
    // FR-010: an unknown is stated as an unknown, never rendered as an agreement.
    await expect(unknown).toContainText('not an agreement')
    await expect(page.getByTestId('guided-compare-table')).toHaveCount(0)
    await expect(page.getByTestId('guided-compare-agrees')).toHaveCount(0)
    await expect(page.getByTestId('guided-compare-not-writable')).toContainText(
      'not provisioned by Fathom',
    )
    await expect(page.getByTestId('guided-next')).toBeEnabled()
  })

  test('switching context under the open path says so and re-reads (FR-026)', async ({ page }) => {
    await compareWith(page, matching)
    await expect(page.getByTestId('guided-compare-context')).toContainText('kind-a')
    await clearSent(page)
    await emit(page, batch([system(50, { type: 'ContextChanged', data: { current: 'kind-b' } })]))
    await expect(page.getByTestId('guided-compare-stale')).toContainText('kind-b')
    // The stale comparison still names the context it came from, not the one now active.
    await expect(page.getByTestId('guided-compare-context')).toContainText('kind-a')
    await expect
      .poll(async () => (await sent(page)).some((c) => c.type === 'ReadApiServerAuth'))
      .toBe(true)
  })
})

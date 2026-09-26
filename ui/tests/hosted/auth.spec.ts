/**
 * Live sign-in lifecycle against the real provider (spec 031 US1, SC-005, SC-007, SC-009):
 * sign-in (PKCE), silent renewal, sign-out, an invalid/expired session cookie, and revocation
 * at the provider.
 */
import { expect, test } from '@playwright/test'

import { APP, apiSession, kubeUser, RawWs, signIn } from './helpers'
import { restoreUser, revokeUser } from './provider'

test('unauthenticated / redirects to the provider with PKCE S256', async ({ page }) => {
  const authorize = page.waitForRequest((r) => /[?&]code_challenge=/.test(r.url()))
  await page.goto('/')
  const url = new URL((await authorize).url())
  expect(url.hostname).not.toBe('fathom.localtest.me')
  expect(url.searchParams.get('code_challenge_method')).toBe('S256')
  expect(url.searchParams.get('response_type')).toBe('code')
  expect(url.searchParams.get('client_id')).toBe('fathom')
  expect(url.searchParams.get('redirect_uri')).toBe('https://fathom.localtest.me/auth/callback')
  expect(url.searchParams.get('state')).toBeTruthy()
  expect(url.searchParams.get('nonce')).toBeTruthy()
})

test('sign-in via the provider lands in the app as that user', async ({ page }) => {
  await signIn(page, 'alice')
  await expect(
    page.getByRole('button', { name: `Account menu for ${kubeUser('alice')}` }),
  ).toBeVisible()
  const s = await apiSession(page)
  expect(s.status).toBe(200)
  expect(s.body.username).toBe(kubeUser('alice'))
  expect(s.body.mode).toBe('hosted')
  expect(s.body.groups).toContain(kubeUser('fathom-viewers'))
  // the session cookie is HttpOnly: invisible to page script
  expect(await page.evaluate(() => document.cookie)).not.toContain('fathom_session')
  const cookie = (await page.context().cookies()).find((c) => c.name === '__Host-fathom_session')
  expect(cookie?.httpOnly).toBe(true)
  expect(cookie?.secure).toBe(true)
  expect(cookie?.sameSite).toBe('Lax')
})

test('silent renewal keeps the live view up (60 s tokens)', async ({ page }) => {
  test.setTimeout(240_000)
  const sockets: string[] = []
  page.on('websocket', (ws) => sockets.push(ws.url()))
  await signIn(page, 'alice')
  await expect(
    page.getByRole('button', { name: `Account menu for ${kubeUser('alice')}` }),
  ).toBeVisible()
  const start = (await apiSession(page)).body.refreshCount ?? 0
  await expect
    .poll(async () => (await apiSession(page)).body.refreshCount ?? 0, {
      timeout: 180_000,
      intervals: [5_000],
    })
    .toBeGreaterThanOrEqual(start + 2)
  // no interruption: still the shell, never the session-ended panel, one socket the whole time
  await expect(
    page.getByRole('button', { name: `Account menu for ${kubeUser('alice')}` }),
  ).toBeVisible()
  await expect(page.getByRole('heading', { name: /Session expired|Access revoked/ })).toHaveCount(0)
  expect(sockets.length).toBe(1)
})

test('sign-out ends the session server-side and returns via the provider', async ({
  page,
  context,
}) => {
  await signIn(page, 'bob')
  // a second tab of the same browser holds its own live socket
  const other = await context.newPage()
  await other.goto('/healthz')
  const raw = await RawWs.open(other)
  const navs: string[] = []
  // navigation requests include every redirect hop (framenavigated only sees the last one)
  page.on('request', (r) => {
    if (r.isNavigationRequest() && r.frame() === page.mainFrame()) navs.push(r.url())
  })
  await page.getByRole('button', { name: `Account menu for ${kubeUser('bob')}` }).click()
  await page.getByRole('menuitem', { name: 'Sign out' }).click()
  // the other tab's socket is closed with 4401 (live views stop everywhere)
  await expect.poll(() => raw.closeCode()).toBe(4401)
  expect((await apiSession(other)).status).toBe(401)
  // RP-initiated logout: the browser goes to the provider's end-session endpoint.
  const isEnd = (u: string) => {
    const url = new URL(u)
    return url.origin !== APP && /logout|end_session/i.test(url.pathname)
  }
  const backAfterEnd = () => {
    const i = navs.findIndex(isEnd)
    return i >= 0 && navs.slice(i + 1).some((u) => u.startsWith(APP))
  }
  let confirmed = false
  try {
    await expect.poll(() => navs.some(isEnd)).toBe(true)
    const hint = new URL(navs.find(isEnd) ?? '').searchParams.has('id_token_hint')
    test.info().annotations.push({ type: 'id_token_hint', description: String(hint) })
    if (hint) {
      // with a hint the provider validates post_logout_redirect_uri and returns to the app,
      // which sends the now signed-out browser to the provider's sign-in page
      await expect.poll(backAfterEnd).toBe(true)
    } else {
      // Without `id_token_hint` (never sent when the ID token is the user's cluster
      // credential: pass-through + kubeToken=id) providers ask the user to confirm and may
      // not return to the app. Confirm once, like a user; returning is recorded, not required.
      const confirm = page.getByRole('button', { name: /^(Logout|Log out|Sign out|Yes)$/i })
      await confirm.click({ timeout: 10_000 }).then(
        () => (confirmed = true),
        () => undefined,
      )
      await expect
        .poll(backAfterEnd, { timeout: 10_000 })
        .toBe(true)
        .catch(() => undefined)
    }
  } finally {
    test
      .info()
      .annotations.push(
        { type: 'navigations', description: navs.map((u) => u.split('?')[0]).join(' -> ') },
        { type: 'provider-confirmation', description: String(confirmed) },
        { type: 'returned-to-app', description: String(backAfterEnd()) },
      )
  }
  await expect(page).not.toHaveURL(/^https:\/\/fathom\.localtest\.me/)
})

test('an expired or forged session cookie is never accepted', async ({ page, context }) => {
  await signIn(page, 'alice')
  const c = (await context.cookies()).find((x) => x.name === '__Host-fathom_session')
  if (!c) throw new Error('no session cookie')
  await context.clearCookies()
  await context.addCookies([{ ...c, value: 'expired-or-forged-session-id' }])
  const s = await apiSession(page)
  expect(s.status).toBe(401)
  expect(s.body.error).toBe('unauthenticated')
  // a reload goes back through sign-in instead of showing any data
  await page.goto('/')
  await expect(page).not.toHaveURL(/^https:\/\/fathom\.localtest\.me/)
})

test.describe('revocation', () => {
  test.afterEach(async () => {
    await restoreUser('bob')
  })

  test('disabling the user at the provider ends their live views within 5 minutes', async ({
    page,
  }) => {
    test.setTimeout(420_000)
    await signIn(page, 'bob')
    await expect(
      page.getByRole('button', { name: `Account menu for ${kubeUser('bob')}` }),
    ).toBeVisible()
    // a second tab on a static page holds a raw socket that outlives the app tab's redirect
    const other = await page.context().newPage()
    await other.goto('/healthz')
    const raw = await RawWs.open(other)
    const t0 = Date.now()
    await revokeUser('bob')
    // The server ends the session (close 4401). The hosted transport then sends the browser
    // back to sign-in, i.e. to the provider, which no longer lets bob in; a page that has not
    // navigated yet shows the session-ended panel instead. Either way the live view is gone.
    // "Identity provider unavailable" (the grace-period path) does not count: that would mean
    // the provider's refusal was misread as an outage.
    await expect
      .poll(
        async () => {
          if (!page.url().startsWith(APP)) return 'sent-to-sign-in'
          const panel = page.getByRole('heading', { name: /Access revoked|Session expired/ })
          return (await panel.count()) > 0 ? 'ended-panel' : 'live'
        },
        { timeout: 300_000, intervals: [2_000] },
      )
      .not.toBe('live')
    const elapsed = (Date.now() - t0) / 1000
    test.info().annotations.push({ type: 'revocation-seconds', description: elapsed.toFixed(1) })
    expect(elapsed).toBeLessThanOrEqual(300)
    await expect.poll(() => raw.closeCode()).toBe(4401)
    const s = await apiSession(other)
    expect(s.status).toBe(401)
  })
})

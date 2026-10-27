/** The sign-in gate and the session over the socket (spec 055 T030/T031). */
import { Close, expect, fixtures, test } from '../../support/fake-backend'

test('signed out: shows sign-in and opens no socket', async ({ page, backend }) => {
  await backend.signedOut()
  await page.goto('/runs')
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  expect(backend.sockets).toHaveLength(0)
})

test('signs in through /auth/login and returns to the page asked for', async ({ page, backend }) => {
  await backend.signedOut()
  backend.loginAs = fixtures.me({ display_name: 'Grace Hopper' })
  await page.goto('/audit')
  await page.getByRole('link', { name: 'Sign in' }).click()

  await expect(page.getByRole('heading', { name: 'Audit' })).toBeVisible()
  await expect(page).toHaveURL(/\/audit$/)
  await expect(page.getByTestId('whoami')).toHaveText('Grace Hopper')
  expect(backend.http.map((c) => c.path)).toContain('/auth/login')
  expect(backend.hellos[0]).toMatchObject({ protocol_version: 1, client: 'keel-web/0.1.0', csrf_token: backend.csrf })
  await backend.waitForCmd('GetMe')
})

test('a cookie without a session lands on sign-in, with no notice', async ({ page, backend }) => {
  backend.me = null
  await page.goto('/runs')
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  await expect(page.locator('[data-notice]')).toHaveCount(0)
  expect(backend.sockets[0]?.closed).toBe(Close.Unauthenticated)
})

test('Revoked signs out with a notice and does not reconnect', async ({ page, backend }) => {
  await page.goto('/runs')
  await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible()
  backend.revoke()
  await expect(page.locator('[data-notice="revoked"]')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  expect(backend.sockets).toHaveLength(1)
})

test('a scripted envelope replay reaches the shell', async ({ page, backend }) => {
  await page.goto('/runs')
  await backend.waitForCmd('GetMe')
  await expect(page.getByTestId('whoami')).toHaveText('Ada Lovelace')
  backend.replay([{ event: { type: 'Me', data: fixtures.me({ display_name: 'Ada K.' }) } }])
  await expect(page.getByTestId('whoami')).toHaveText('Ada K.')
})

test('the session ending under the page shows the expired notice', async ({ page, backend }) => {
  await page.goto('/runs')
  await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible()
  backend.me = null
  backend.closeSockets(Close.Unauthenticated)
  await expect(page.locator('[data-notice="expired"]')).toBeVisible()
})

test('a dropped socket reconnects and asks GetMe again', async ({ page, backend }) => {
  await page.goto('/runs')
  await backend.waitForCmd('GetMe')
  backend.closeSockets(Close.GoingAway)
  await backend.waitForCmd('GetMe')
  await expect(page.getByRole('status').filter({ hasText: 'Reconnecting' })).toHaveCount(0)
  expect(backend.sockets).toHaveLength(2)
  await expect(page.getByRole('heading', { name: 'Runs' })).toBeVisible()
})

test('another protocol version shows the outdated notice', async ({ page, backend }) => {
  backend.helloClose = Close.Outdated
  await page.goto('/runs')
  await expect(page.locator('[data-notice="outdated"]')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Reload' })).toBeVisible()
})

test('sign-out posts the CSRF token and shows sign-in', async ({ page, backend }) => {
  await page.goto('/runs')
  await page.getByRole('button', { name: 'Account' }).click()
  await page.getByRole('menuitem', { name: 'Sign out' }).click()
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  expect(backend.http).toContainEqual({ method: 'POST', path: '/auth/logout', csrf: backend.csrf })
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
})

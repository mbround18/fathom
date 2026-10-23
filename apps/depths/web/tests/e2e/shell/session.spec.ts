/**
 * The shell's sign-in gate and transport, end to end against the fake backend (T043, T044).
 */
import { CloseCode, PROTOCOL_VERSION } from '../../../src/protocol/generated/constants'
import { expect, fixtures as f, sessions, test, type FakeBackend } from '../../support/fake-backend'

/** Waits for `n` sockets to have said a good Hello and stay open. The indicator hides when open. */
async function connected(backend: FakeBackend, n = 1): Promise<void> {
  await expect.poll(() => backend.sockets.filter((s) => s.ready && s.closed === null).length).toBe(n)
}

test('a 401 shows sign-in with the configured providers and opens no socket', async ({ page, backend }) => {
  backend.me = null
  backend.providers = ['github', 'oidc']
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  await expect(page.getByRole('link', { name: 'Sign in with GitHub' })).toHaveAttribute(
    'href',
    '/auth/login?provider=github',
  )
  await expect(page.getByRole('link', { name: 'Sign in with your identity provider' })).toHaveAttribute(
    'href',
    '/auth/login?provider=oidc',
  )
  await expect(page.getByRole('link', { name: 'Sign in with Discord' })).toHaveCount(0)
  expect(backend.sockets).toHaveLength(0)
})

test('a pending person sees the no-access page and nothing about the host', async ({ page, backend }) => {
  backend.me = sessions.pending()
  backend.stacks = f.stackList([f.stack('secret-stack')])
  await page.goto('/')
  await expect(page.getByRole('heading', { name: "You don't currently have access" })).toBeVisible()
  await expect(page.getByText('secret-stack')).toHaveCount(0)
  await expect(page.getByText('/srv/stacks')).toHaveCount(0)
  expect(backend.sockets).toHaveLength(0)
})

test('a user with stacks gets the shell, and Hello is the first frame', async ({ page, backend }) => {
  backend.me = sessions.user(['web'], { csrf_token: 'csrf-e2e' })
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Stacks' })).toBeVisible()
  await connected(backend)
  expect(backend.hellos).toEqual([{ protocol_version: PROTOCOL_VERSION, csrf_token: 'csrf-e2e' }])
})

test('a dropped socket reconnects and says Hello again', async ({ page, backend }) => {
  await page.goto('/')
  await connected(backend)
  backend.closeSockets(1006)
  await expect.poll(() => backend.sockets.length).toBe(2)
  await connected(backend)
  await expect(page.locator('[data-connection]')).toHaveCount(0)
  expect(backend.hellos).toHaveLength(2)
})

test('a 4401 close routes to sign-in', async ({ page, backend }) => {
  await page.goto('/')
  await connected(backend)
  backend.me = null
  backend.closeSockets(CloseCode.Unauthenticated)
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  await expect(page.getByRole('link', { name: 'Sign in with GitHub' })).toBeVisible()
  expect(backend.sockets).toHaveLength(1)
})

test('a 4403 close routes to no-access', async ({ page, backend }) => {
  await page.goto('/')
  await connected(backend)
  backend.closeSockets(CloseCode.Forbidden)
  await expect(page.getByRole('heading', { name: "You don't currently have access" })).toBeVisible()
  expect(backend.sockets).toHaveLength(1)
})

test('sign-out sends the CSRF token and lands on sign-in', async ({ page, backend }) => {
  const token = backend.me?.csrf_token
  await page.goto('/')
  await page.getByRole('button', { name: 'Sign out' }).click()
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible()
  const logout = backend.http.find((c) => c.path === '/auth/logout')
  expect(logout).toEqual({ method: 'POST', path: '/auth/logout', csrf: token })
})

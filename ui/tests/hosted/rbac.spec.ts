/**
 * Per-user RBAC enforced by the API server itself (spec 031 US1, SC-002, SC-008): alice
 * (fathom-viewers: view) reads but a forced mutation is refused verbatim; bob (fathom-editors:
 * edit in `demo`) mutates in `demo` and is refused elsewhere; carol (no group) sees the
 * no-access state within 3 s.
 */
import { expect, test } from '@playwright/test'

import { kubectl, kubeUser, providerLogin, RawWs, signIn } from './helpers'

const replicas = (): string =>
  kubectl('-n', 'demo', 'get', 'deploy', 'rbac-probe', '-o', 'jsonpath={.spec.replicas}').trim()

test('alice can read but a forced scale is refused by the API server, verbatim', async ({
  page,
}) => {
  await signIn(page, 'alice')
  await expect(
    page.getByRole('button', { name: `Account menu for ${kubeUser('alice')}` }),
  ).toBeVisible()
  const raw = await RawWs.open(page)
  const read = await raw.command({
    type: 'SelectDeployment',
    data: { namespace: 'demo', name: 'rbac-probe' },
  })
  expect(read.ok, read.error ?? '').toBe(true)
  const before = replicas()
  const scale = await raw.command({
    type: 'ScaleDeployment',
    data: { namespace: 'demo', name: 'rbac-probe', replicas: 3 },
  })
  expect(scale.ok).toBe(false)
  // the Kubernetes Forbidden text, naming the OIDC-mapped user, passed through unchanged
  expect(scale.error).toMatch(/forbidden/i)
  expect(scale.error).toContain('User "oidc:alice"')
  expect(replicas()).toBe(before)
})

test('bob can scale in demo and is refused in kube-system', async ({ page }) => {
  await signIn(page, 'bob')
  await expect(
    page.getByRole('button', { name: `Account menu for ${kubeUser('bob')}` }),
  ).toBeVisible()
  const raw = await RawWs.open(page)
  // bob can only list in demo: scope the view there first (the post-mutation list refresh
  // follows the namespace filter, as when the user picks the namespace in the UI)
  const scoped = await raw.command({ type: 'SetNamespaceFilter', data: { namespaces: ['demo'] } })
  expect(scoped.ok, scoped.error ?? '').toBe(true)
  // the no-access probe must see bob's RoleBinding in demo (accessProbeNamespaces)
  await expect(page.getByRole('heading', { name: 'No access' })).toHaveCount(0)
  try {
    const up = await raw.command({
      type: 'ScaleDeployment',
      data: { namespace: 'demo', name: 'rbac-probe', replicas: 2 },
    })
    expect(up.ok, up.error ?? '').toBe(true)
    expect(replicas()).toBe('2')
  } finally {
    await raw.command({
      type: 'ScaleDeployment',
      data: { namespace: 'demo', name: 'rbac-probe', replicas: 1 },
    })
  }
  expect(replicas()).toBe('1')
  const other = await raw.command({
    type: 'ScaleDeployment',
    data: { namespace: 'kube-system', name: 'coredns', replicas: 3 },
  })
  expect(other.ok).toBe(false)
  expect(other.error).toMatch(/forbidden/i)
  expect(other.error).toContain('User "oidc:bob"')
})

test('carol (no permissions) sees the no-access state within 3 s', async ({ page }) => {
  await page.goto('/')
  await providerLogin(page, 'carol')
  await page.waitForURL(
    (u) => u.hostname === 'fathom.localtest.me' && !u.pathname.startsWith('/auth/'),
    {
      waitUntil: 'commit',
    },
  )
  const t0 = Date.now()
  await expect(page.getByRole('heading', { name: 'No access' })).toBeVisible({ timeout: 10_000 })
  const ms = Date.now() - t0
  test.info().annotations.push({ type: 'no-access-ms', description: String(ms) })
  expect(ms).toBeLessThanOrEqual(3_000)
  await expect(page.getByText(/carol is signed in/)).toBeVisible()
})

test('hosted mode refuses local-only commands (filter is the boundary, not the UI)', async ({
  page,
}) => {
  await signIn(page, 'bob')
  const raw = await RawWs.open(page)
  const r = await raw.command({ type: 'SetContext', data: { context: 'kind-anything' } })
  expect(r.ok).toBe(false)
  expect(r.error).toMatch(/Forbidden|not available/i)
})

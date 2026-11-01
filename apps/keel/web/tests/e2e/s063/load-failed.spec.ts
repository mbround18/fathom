/**
 * A list that fails to load says so, and why: "Failed to load {what}." with the server's reason,
 * in place of the loading and empty states, and a Retry that loads it again. Against the fake
 * backend, with the list request failing until the test lets it through.
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance } from '../../support/fake-governance'
import { FakeHosts } from '../../support/fake-hosts'
import { WORKSPACE } from '../../support/fake-repos'
import { FakeReviews } from '../../support/fake-reviews'
import { FakeRuns } from '../../support/fake-runs'

/** A workspace admin on the one workspace, with every list in it empty. */
function admin(backend: FakeBackend): void {
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  governance(backend, (g) => {
    g.workspaces.push({ id: WORKSPACE, slug: 'ops', name: 'Ops', created: '2026-10-27T12:00:00Z', archived: false, projects: [], system: false })
  })
  new FakeRuns(backend)
  new FakeReviews(backend).reviews = []
  new FakeHosts(backend)
}

const LISTS = [
  { path: '/runs', cmd: 'ListRuns', what: 'runs', empty: 'runs' },
  { path: '/hosts', cmd: 'ListHosts', what: 'hosts', empty: 'hosts' },
  { path: '/bindings', cmd: 'ListBindings', what: 'bindings', empty: 'bindings' },
] as const

for (const { path, cmd, what, empty } of LISTS) {
  test(`${what}: a failed load says why, and Retry loads it`, async ({ page, backend }) => {
    admin(backend)
    let fail = true
    backend.chain(cmd, () =>
      fail ? [{ type: 'OpError', data: { code: 'internal', message: `The ${what} store is down.` } }] : undefined,
    )

    await page.goto(path)
    const failed = page.locator('[data-pane-state="error"]')
    await expect(failed).toContainText(`Failed to load ${what}.`)
    await expect(failed).toContainText(`The ${what} store is down.`)
    await expect(failed.locator('[data-error-code]')).toHaveText('internal')
    await expect(page.locator(`[data-empty-page="${empty}"]`)).toHaveCount(0)
    await expect(page.getByRole('status').filter({ hasText: 'Loading' })).toHaveCount(0)

    fail = false
    await failed.getByRole('button', { name: 'Retry' }).click()
    await expect(page.locator(`[data-empty-page="${empty}"]`)).toBeVisible()
    await expect(failed).toHaveCount(0)
  })
}

test('a denied list shows the action and reason the server gave', async ({ page, backend }) => {
  admin(backend)
  backend.chain('ListRuns', () => [{ type: 'Denied', data: { action: 'list_runs', reason: 'no_grant' } }])
  await page.goto('/runs')
  const failed = page.locator('[data-pane-state="error"]')
  await expect(failed).toContainText('Failed to load runs.')
  await expect(failed.locator('[data-denied-action]')).toHaveText('list_runs')
  await expect(failed.locator('[data-denied-reason]')).toHaveText('no_grant')
  await expect(page.locator('[data-empty]')).toHaveCount(0)
})

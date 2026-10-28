/**
 * Spec 058 amendment A2 (T039): a host whose runner reports an `ansible-playbook` run outside Keel
 * shows "Changed outside Keel" live, on its row and in its view, with the entries labelled as the
 * host's claims; the list filters on it; a baseline approval clears it. Against the fake backend
 * (tests/support/fake-hosts.ts).
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'
import { FakeHosts } from '../../support/fake-hosts'

function admin(backend: FakeBackend): FakeHosts {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  return new FakeHosts(backend)
}

test('a run outside Keel flags the host live, filters, and a baseline clears it', async ({ page, backend }) => {
  const hosts = admin(backend)
  hosts.connect('web-1')
  hosts.connect('db-1')

  await page.goto('/hosts')
  const table = page.getByRole('table', { name: 'Hosts' })
  const web = table.locator('tr', { hasText: 'web-1' })
  const db = table.locator('tr', { hasText: 'db-1' })
  await expect(web).toHaveAttribute('data-host-state', 'online')
  await expect(table.getByText('Changed outside Keel')).toHaveCount(0)

  // Open web-1's view before anything is reported: no claims yet.
  await web.getByRole('button', { name: 'Show web-1' }).click()
  const view = page.getByRole('region', { name: 'Host web-1' })
  await expect(view).toContainText('The host has reported none.')

  // A `--check` run is listed but does not flag the host.
  hosts.reportOutside('web-1', { at: '2026-10-27T10:00:00Z', playbook: 'site.yaml', check: true, dirty: false })
  await expect(web).toHaveAttribute('data-host-changed-outside', 'false')

  // A real run flags it, live, on the row and in the open view.
  hosts.reportOutside('web-1', {
    at: '2026-10-27T11:00:00Z',
    user: 'deploy',
    playbook: 'playbooks/debian.yaml',
    commit: 'a'.repeat(40),
  })
  await expect(web.getByText('Changed outside Keel')).toBeVisible()
  await expect(web).toHaveAttribute('data-host-changed-outside', 'true')
  await expect(db).toHaveAttribute('data-host-changed-outside', 'false')
  await expect(view.getByText('Changed outside Keel')).toBeVisible()
  await expect(view).toContainText("These are the host's claims")
  const runs = view.getByRole('list', { name: 'Runs outside Keel' }).getByRole('listitem')
  await expect(runs).toHaveCount(2)
  await expect(runs.nth(0)).toContainText('playbooks/debian.yaml')
  await expect(runs.nth(0)).toContainText('host says: deploy')
  await expect(runs.nth(0)).toContainText('a'.repeat(12))
  await expect(runs.nth(0)).toContainText('uncommitted changes')
  await expect(runs.nth(1)).toHaveAttribute('data-outside-check', 'true')
  await expect(runs.nth(1)).toContainText('--check')

  // The filter narrows the list to flagged hosts, on the server's answer.
  await page.getByLabel('Only hosts changed outside Keel').check()
  await expect
    .poll(() => backend.received.findLast((c) => c.type === 'ListHosts')?.data)
    .toMatchObject({ changed_outside: true })
  await expect(db).toHaveCount(0)
  await expect(web).toBeVisible()

  // A baseline approval clears it: under the filter the host leaves the list, and the view says so.
  hosts.approveBaseline('web-1')
  await expect(web).toHaveCount(0)
  await expect(table).toContainText('No host reports a run outside Keel.')
  await expect(view.getByText('Changed outside Keel')).toHaveCount(0)
  await expect(runs).toHaveCount(2)

  await page.getByLabel('Only hosts changed outside Keel').uncheck()
  await expect(web).toHaveAttribute('data-host-changed-outside', 'false')
  await expect(db).toBeVisible()
})

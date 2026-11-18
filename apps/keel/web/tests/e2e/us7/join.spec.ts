/**
 * US7 (spec 055 T148, contracts/join.md): join tokens and join requests, against the fake backend.
 *
 * - Hosts → Join requests: a host waiting shows its key fingerprint beside the instruction to
 *   compare it; accept is prefilled and refuses profiles the host does not allow; confirmed, the
 *   request leaves the pending list. A request pushed while the tab is open appears without a
 *   reload, and is rejected with a note.
 * - Settings → Hosts: a join token is issued and shown once, masked, with the claims a host will
 *   read; Done drops it for good; Disable leaves the workspace without one.
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { WORKSPACE } from '../../support/fake-repos'
import { FakeHosts } from '../../support/fake-hosts'
import type { Event, JoinRequestInfo, JoinTokenInfo } from '../../../src/protocol/wire'

const EXPIRES = '2099-01-01T00:00:00Z'
const FINGERPRINT = 'SHA256:q0nJ8m1h2pY9cX3s4dV6tW7uZ0aB1eF2gH3iJ4kL5mN'

function operator(backend: FakeBackend): void {
  backend.me = fixtures.me({
    instance_role: 'user',
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  backend.on('ListWorkspaces', () => [
    {
      type: 'Workspaces',
      data: {
        items: [
          { id: WORKSPACE, slug: 'ops', name: 'Ops', created: '2026-10-27T12:00:00Z', archived: false, projects: [], system: false, primary_binding: null },
        ],
      },
    },
  ])
  new FakeHosts(backend)
}

function request(over: Partial<JoinRequestInfo>): JoinRequestInfo {
  return {
    id: 'jr-1',
    workspace: WORKSPACE,
    fingerprint: FINGERPRINT,
    inventory_name: 'edge-01',
    allowed_profiles: ['base', 'web'],
    suggested_profiles: ['web'],
    require_signed: true,
    hostname: 'edge-01.lan',
    os: 'Debian GNU/Linux 13 (trixie)',
    arch: 'x86_64',
    runner_version: '0.1.0',
    source: '192.0.2.10',
    state: 'pending',
    reason: null,
    comment: null,
    jti: 'jti-1',
    first_seen_at: '2026-11-17T09:00:00Z',
    last_seen_at: '2026-11-17T09:30:00Z',
    decided_by: null,
    decided_at: null,
    host: null,
    ...over,
  }
}

function preview(preview_id: string, summary: string): Event {
  return { type: 'Preview', data: { preview_id, action: preview_id, summary, affected: [], step_up: true, expires_at: EXPIRES } }
}

function b64url(value: unknown): string {
  return Buffer.from(JSON.stringify(value)).toString('base64url')
}

test('a host waiting to join is checked by fingerprint, accepted, and another rejected', async ({ page, backend }) => {
  operator(backend)
  let requests = [request({})]
  backend.on('ListJoinRequests', (cmd) => [
    { type: 'JoinRequests', data: { items: requests.filter((r) => cmd.data.state === null || r.state === cmd.data.state) } },
  ])
  backend.on('PreviewJoinAccept', (cmd) => [preview('accept-1', `Accept ${cmd.data.inventory_name} into Ops.`)])
  backend.on('PreviewJoinReject', () => [preview('reject-1', 'Reject the request.')])
  backend.chain('Confirm', (cmd) => {
    const id = cmd.data.preview_id
    if (id !== 'accept-1' && id !== 'reject-1') return undefined
    const target = id === 'accept-1' ? 'jr-1' : 'jr-2'
    const decided = requests.find((r) => r.id === target)
    if (!decided) return undefined
    const updated: JoinRequestInfo =
      id === 'accept-1'
        ? { ...decided, state: 'accepted', inventory_name: 'edge-1', host: 'h-9' }
        : { ...decided, state: 'rejected', reason: 'rejected', comment: 'Not ours.' }
    requests = requests.map((r) => (r.id === target ? updated : r))
    return [
      { type: 'Confirmed', data: { preview_id: id } },
      { type: 'JoinRequestDecided', data: { request: updated } },
    ]
  })

  await page.goto('/hosts?tab=join')
  expect((await backend.waitForCmd('ListJoinRequests')).data).toEqual({ workspace: WORKSPACE, state: 'pending' })
  const list = page.getByRole('list', { name: 'Join requests' })
  const card = list.locator('[data-join-request="jr-1"]')
  await expect(card).toHaveAttribute('data-join-state', 'pending')
  await expect(card.locator('[data-copy="join-fingerprint"]')).toHaveText(FINGERPRINT)
  await expect(card).toContainText('keel-runner fingerprint')
  await expect(card).toContainText('Debian GNU/Linux 13 (trixie)')

  // Accept: prefilled with the host's name and the token's profiles; a profile the host refuses
  // is caught before Keel sees it.
  await card.getByRole('button', { name: 'Accept edge-01.lan' }).click()
  const form = card.getByRole('form', { name: 'Accept join request' })
  await expect(form.getByLabel('Inventory name')).toHaveValue('edge-01')
  await expect(form.getByLabel('Profiles')).toHaveValue('web')
  await form.getByLabel('Profiles').fill('web, db')
  await expect(form).toContainText('The host does not allow db.')
  await expect(form.getByRole('button', { name: 'Accept' })).toBeDisabled()
  await form.getByLabel('Profiles').fill('base, web')
  await form.getByLabel('Inventory name').fill('edge-1')
  // Converge after accepting is on by default and is sent as the default (field left out).
  await expect(form.getByRole('checkbox', { name: 'Converge after accepting' })).toBeChecked()
  await form.getByRole('button', { name: 'Accept' }).click()
  expect((await backend.waitForCmd('PreviewJoinAccept')).data).toEqual({
    request: 'jr-1',
    inventory_name: 'edge-1',
    profiles: ['base', 'web'],
  })
  const dialog = page.getByRole('dialog')
  await expect(dialog).toContainText('Accept edge-1 into Ops.')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(card).toHaveCount(0)
  await expect(page.getByText('No host is waiting')).toBeVisible()

  // Another host asks while the tab is open: it shows up without a reload, and is rejected.
  const second = request({ id: 'jr-2', hostname: 'stray', inventory_name: 'stray', fingerprint: 'SHA256:other', last_seen_at: '2026-11-17T10:00:00Z' })
  requests.push(second)
  backend.push({ type: 'JoinRequested', data: { request: second } })
  const stray = list.locator('[data-join-request="jr-2"]')
  await expect(stray).toHaveAttribute('data-join-state', 'pending')
  await stray.getByRole('button', { name: 'Reject stray' }).click()
  const reject = stray.getByRole('form', { name: 'Reject join request' })
  await reject.getByLabel('Note (optional)').fill('  Not ours.  ')
  await reject.getByRole('button', { name: 'Reject' }).click()
  expect((await backend.waitForCmd('PreviewJoinReject')).data).toEqual({ request: 'jr-2', comment: 'Not ours.' })
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(stray).toHaveCount(0)

  // Every state: the decisions are kept, with why.
  await page.getByLabel('State').selectOption('')
  await expect(list.locator('[data-join-request="jr-1"]')).toHaveAttribute('data-join-state', 'accepted')
  const rejected = list.locator('[data-join-request="jr-2"]')
  await expect(rejected).toHaveAttribute('data-join-state', 'rejected')
  await expect(rejected.locator('[data-field="reason"]')).toHaveText('Rejected by an operator')
  await expect(rejected.locator('[data-field="comment"]')).toHaveText('Not ours.')
  await expect(rejected.getByRole('button', { name: /Accept|Reject/ })).toHaveCount(0)
})

test('accepting with Converge after accepting unticked asks Keel not to converge', async ({ page, backend }) => {
  operator(backend)
  backend.on('ListJoinRequests', () => [{ type: 'JoinRequests', data: { items: [request({})] } }])
  backend.on('PreviewJoinAccept', (cmd) => [preview('accept-1', `Accept ${cmd.data.inventory_name} into Ops.`)])

  await page.goto('/hosts?tab=join')
  const card = page.getByRole('list', { name: 'Join requests' }).locator('[data-join-request="jr-1"]')
  await card.getByRole('button', { name: 'Accept edge-01.lan' }).click()
  const form = card.getByRole('form', { name: 'Accept join request' })
  const converge = form.getByRole('checkbox', { name: 'Converge after accepting' })
  await expect(form).toContainText('Once accepted, Keel runs the steps for web on it')
  await converge.click()
  await expect(converge).not.toBeChecked()
  await expect(form).toContainText('Keel waits for someone to converge it.')
  await form.getByRole('button', { name: 'Accept' }).click()
  expect((await backend.waitForCmd('PreviewJoinAccept')).data).toEqual({
    request: 'jr-1',
    inventory_name: 'edge-01',
    profiles: ['web'],
    auto_converge: false,
  })
})

test('a join token is issued and shown once, then disabled', async ({ page, context, backend }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  operator(backend)
  const none: JoinTokenInfo = {
    workspace: WORKSPACE,
    active: false,
    jti: null,
    expires_at: null,
    created_at: null,
    created_by: null,
    inventory_name: null,
    profiles: [],
  }
  let current = none
  const claims = {
    iss: 'https://keel.example',
    aud: 'keel-runner',
    sub: WORKSPACE,
    jti: 'jti-7',
    iat: 1_800_000_000,
    exp: 1_800_604_800,
    'keel.server': 'keel.example:7443',
    'keel.host_ca': 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHostCa keel-host-ca',
    'keel.origin': 'https://keel.example',
    'keel.rp_id': 'keel.example',
    'keel.profiles': ['base'],
  }
  const token = `${b64url({ alg: 'EdDSA', kid: 'k1', typ: 'JWT' })}.${b64url(claims)}.c2lnbmF0dXJl`
  backend.on('GetJoinToken', () => [{ type: 'JoinToken', data: current }])
  backend.on('PreviewJoinTokenIssue', () => [preview('issue-1', 'Issue a join token for Ops.')])
  backend.on('PreviewJoinTokenDisable', () => [preview('disable-1', 'Disable the join token for Ops.')])
  backend.chain('Confirm', (cmd) => {
    const id = cmd.data.preview_id
    if (id === 'issue-1') {
      current = {
        ...none,
        active: true,
        jti: 'jti-7',
        expires_at: '2027-01-22T08:00:00Z',
        created_at: '2027-01-15T08:00:00Z',
        created_by: 'Ana',
        profiles: ['base'],
      }
      return [
        { type: 'Confirmed', data: { preview_id: id } },
        { type: 'JoinTokenSecret', data: { info: current, token, rejected: 0 } },
      ]
    }
    if (id === 'disable-1') {
      current = none
      return [
        { type: 'Confirmed', data: { preview_id: id } },
        { type: 'JoinToken', data: current },
      ]
    }
    return undefined
  })

  await page.goto('/settings/hosts')
  const pane = page.locator('[data-pane="join-token"]')
  await expect(pane.locator('[data-join-token-state="none"]')).toBeVisible()

  const form = pane.getByRole('form', { name: 'Issue join token' })
  await form.getByLabel('Lasts').selectOption('7')
  await form.getByLabel('Profiles (optional)').fill('base')
  await form.getByRole('button', { name: 'Issue join token' }).click()
  expect((await backend.waitForCmd('PreviewJoinTokenIssue')).data).toEqual({
    workspace: WORKSPACE,
    expires_in_days: 7,
    inventory_name: null,
    profiles: ['base'],
  })
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()

  // Once, masked, with what a host will read from it.
  const secret = pane.locator('[data-join-secret]')
  await expect(secret).toContainText('will not show it again')
  const value = secret.locator('[data-copy="join-token"]')
  await expect(value).toHaveAttribute('data-masked', 'true')
  await secret.getByRole('button', { name: 'Copy join token' }).click()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(token)
  await expect(secret).toContainText('keel.example:7443')
  await expect(secret).toContainText('keel-host-ca')
  await expect(pane.locator('[data-field="join-token-state"]')).toHaveText('active')

  await secret.getByRole('button', { name: 'Done' }).click()
  await expect(secret).toHaveCount(0)
  await page.reload()
  await expect(pane.locator('[data-field="join-token-state"]')).toHaveText('active')
  await expect(pane.locator('[data-copy="join-token"]')).toHaveCount(0)

  await pane.getByRole('button', { name: 'Disable' }).click()
  await expect(dialog).toContainText('Disable the join token for Ops.')
  await dialog.getByRole('button', { name: 'Confirm' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click()
  await expect(pane.locator('[data-join-token-state="none"]')).toBeVisible()
})

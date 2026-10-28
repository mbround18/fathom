import { beforeEach, describe, expect, it } from 'vitest'

import { HOST_NAME, useEnroll } from '@/features/hosts/enroll-store'
import { parseProfiles, useHosts } from '@/features/hosts/store'
import type { HostInfo, Preview } from '@/protocol/wire'

import type { FakeSocket } from './fake-socket'
import { frames, lastCmd, resetSession, signIn } from './session-helpers'

function answer(s: FakeSocket, type: string, ...events: unknown[]) {
  const cmd = lastCmd(s, type)
  if (!cmd) throw new Error(`no ${type} was sent`)
  for (const event of events) s.serverSend({ reply_to: cmd.id, event })
}

const TOKEN = `keel-enroll-v1.${'a'.repeat(64)}.SHA256:abc`

const host = (over: Partial<HostInfo> = {}): HostInfo => ({
  id: 'h-1',
  workspace: 'w-1',
  inventory_name: 'web-1',
  state: 'offline',
  profiles: ['base'],
  allowed_profiles: ['base'],
  require_signed: false,
  runner_version: null,
  ansible_version: null,
  enrolled_at: '2026-10-27T12:00:00Z',
  last_seen_at: null,
  revoked_at: null,
  changed_outside: false,
  ...over,
})

const preview = (over: Partial<Preview> = {}): Preview => ({
  preview_id: 'p-1',
  action: 'preview_host_enroll',
  summary: 'Enrol web-1',
  affected: ['web-1'],
  step_up: false,
  expires_at: '2026-10-27T12:15:00Z',
  ...over,
})

beforeEach(() => {
  resetSession()
  useHosts.setState({
    workspace: null,
    hosts: [],
    ssh: null,
    preview: null,
    error: null,
    changedOnly: false,
    open: null,
    detail: null,
  })
  useEnroll.getState().close()
})

describe('helpers', () => {
  it('splits profiles on commas and spaces, once each', () => {
    expect(parseProfiles(' base, web  web,,db ')).toEqual(['base', 'web', 'db'])
    expect(parseProfiles('   ')).toEqual([])
  })
  it('checks inventory names the way keel-protocol does', () => {
    expect(HOST_NAME.test('web-01.example_a')).toBe(true)
    expect(HOST_NAME.test('-web')).toBe(false)
    expect(HOST_NAME.test('web 1')).toBe(false)
    expect(HOST_NAME.test('a'.repeat(253))).toBe(true)
    expect(HOST_NAME.test('a'.repeat(254))).toBe(false)
  })
})

describe('hosts', () => {
  it('lists hosts and the CA line, and follows pushed state', () => {
    const s = signIn()
    useHosts.getState().load('w-1')
    expect(lastCmd(s, 'ListHosts')?.cmd?.data).toEqual({ workspace: 'w-1', profile: null, drift: null, changed_outside: null })
    answer(s, 'ListHosts', { type: 'Hosts', data: { items: [host()] } })
    answer(s, 'GetSshCa', { type: 'SshCa', data: { cas: [], cert_authority: '@cert-authority * ssh-ed25519 AAAA' } })
    expect(useHosts.getState().hosts).toHaveLength(1)
    expect(useHosts.getState().ssh?.cert_authority).toContain('@cert-authority')

    s.serverSend({ event: { type: 'HostState', data: { host: 'h-1', state: 'online' } } })
    expect(useHosts.getState().hosts[0]?.state).toBe('online')

    // A host the list has not seen re-lists, and that answer is applied.
    s.serverSend({ event: { type: 'HostState', data: { host: 'h-2', state: 'online' } } })
    answer(s, 'ListHosts', {
      type: 'Hosts',
      data: { items: [host({ state: 'online' }), host({ id: 'h-2', inventory_name: 'web-2', state: 'online' })] },
    })
    expect(useHosts.getState().hosts.map((h) => h.id)).toEqual(['h-1', 'h-2'])
  })

  it('the Enroll dialog holds a minted token only when it answers its confirm, until closed', () => {
    const s = signIn()
    useEnroll.getState().open('w-1')
    expect(useEnroll.getState().phase).toBe('form')
    const minted = {
      id: 'e-1',
      workspace: 'w-1',
      inventory_name: 'web-1',
      profiles: ['base'],
      expires_at: '2026-10-28T12:15:00Z',
      token: TOKEN,
      snippet: `ansible-playbook -e keel_enroll_token=${TOKEN}`,
      instructions: '',
      instructions_unavailable: false,
    }
    // A token pushed without a reply_to of ours is ignored.
    s.serverSend({ event: { type: 'HostEnrollment', data: { ...minted, id: 'e-0' } } })
    expect(useEnroll.getState().minted).toBeNull()

    useEnroll.getState().submit('w-1', 'web-1', ['base'])
    expect(useEnroll.getState().phase).toBe('asking')
    expect(lastCmd(s, 'PreviewHostEnroll')?.cmd?.data).toEqual({
      workspace: 'w-1',
      inventory_name: 'web-1',
      profiles: ['base'],
    })
    answer(s, 'PreviewHostEnroll', { type: 'Preview', data: preview() })
    expect(useEnroll.getState().phase).toBe('preview')
    void useEnroll.getState().confirm()
    expect(lastCmd(s, 'Confirm')?.cmd?.data).toEqual({ preview_id: 'p-1' })
    const before = useEnroll.getState().mintedCount
    answer(s, 'Confirm', { type: 'Confirmed', data: { preview_id: 'p-1' } }, { type: 'HostEnrollment', data: minted })
    expect(useEnroll.getState().phase).toBe('result')
    expect(useEnroll.getState().minted?.token).toBe(TOKEN)
    expect(useEnroll.getState().mintedCount).toBe(before + 1)

    useEnroll.getState().close()
    expect(useEnroll.getState().minted).toBeNull()
    expect(useEnroll.getState().phase).toBe('closed')
  })

  it('an enrol refusal is shown, and Back returns to the form', () => {
    const s = signIn()
    useEnroll.getState().open('w-1')
    useEnroll.getState().submit('w-1', 'web-1', ['nope'])
    answer(s, 'PreviewHostEnroll', {
      type: 'OpError',
      data: { code: 'profile_not_allowed', message: 'nope is not an allowed profile' },
    })
    expect(useEnroll.getState().phase).toBe('refused')
    expect(useEnroll.getState().refusal).toEqual({
      kind: 'error',
      code: 'profile_not_allowed',
      message: 'nope is not an allowed profile',
    })
    useEnroll.getState().back()
    expect(useEnroll.getState().phase).toBe('form')
  })

  it('a roll previews PreviewEnrollmentRoll and ends on the token', () => {
    const s = signIn()
    useEnroll.getState().roll('e-1')
    expect(useEnroll.getState().mode).toBe('roll')
    expect(lastCmd(s, 'PreviewEnrollmentRoll')?.cmd?.data).toEqual({ enrollment: 'e-1' })
  })

  it('a revoke is previewed, and a refusal is shown', () => {
    const s = signIn()
    useHosts.getState().load('w-1')
    useHosts.getState().revoke('h-1')
    expect(lastCmd(s, 'PreviewHostRevoke')?.cmd?.data).toEqual({ host: 'h-1' })
    answer(s, 'PreviewHostRevoke', { type: 'Denied', data: { action: 'preview_host_revoke', reason: 'no_grant' } })
    expect(useHosts.getState().error).toBe('Not allowed: no_grant')
  })

  it('filters on changed outside Keel and re-reads on HostChangedOutside (058 A2)', () => {
    const s = signIn()
    useHosts.getState().load('w-1')
    answer(s, 'ListHosts', { type: 'Hosts', data: { items: [host()] } })

    useHosts.getState().setChangedOnly(true)
    expect(lastCmd(s, 'ListHosts')?.cmd?.data).toEqual({
      workspace: 'w-1',
      profile: null,
      drift: null,
      changed_outside: true,
    })
    answer(s, 'ListHosts', { type: 'Hosts', data: { items: [] } })
    expect(useHosts.getState().hosts).toEqual([])

    // The open detail and the list are both re-read when the flag moves.
    useHosts.getState().show('h-1')
    expect(lastCmd(s, 'GetHost')?.cmd?.data).toEqual({ host: 'h-1' })
    const flagged = host({ changed_outside: true })
    const run = {
      at: '2026-10-27T11:00:00Z',
      user: 'root',
      playbook: 'site.yaml',
      commit: null,
      dirty: true,
      check: false,
      received_at: '2026-10-27T11:01:00Z',
    }
    const before = frames(s).length
    s.serverSend({ event: { type: 'HostChangedOutside', data: { host: 'h-1' } } })
    const types = frames(s).slice(before).map((f) => f.cmd?.type)
    expect(types).toEqual(['ListHosts', 'GetHost'])
    answer(s, 'GetHost', {
      type: 'Host',
      data: { host: flagged, python_version: null, override_quorum: null, ledger_head: null, certs: [], outside_runs: [run] },
    })
    expect(useHosts.getState().detail?.outside_runs).toEqual([run])
    expect(useHosts.getState().hosts.map((h) => h.changed_outside)).toEqual([true])

    // Cleared, under the filter: it leaves the list.
    answer(s, 'GetHost', {
      type: 'Host',
      data: { host: host(), python_version: null, override_quorum: null, ledger_head: null, certs: [], outside_runs: [run] },
    })
    expect(useHosts.getState().hosts).toEqual([])
    expect(useHosts.getState().detail?.host.changed_outside).toBe(false)

    useHosts.getState().show(null)
    expect(useHosts.getState().detail).toBeNull()
  })
})

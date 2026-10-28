import { beforeEach, describe, expect, it } from 'vitest'

import { parseProfiles, tokenArg, useHosts } from '@/features/hosts/store'
import type { HostInfo, Preview } from '@/protocol/wire'

import type { FakeSocket } from './fake-socket'
import { lastCmd, resetSession, signIn } from './session-helpers'

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
  useHosts.setState({ workspace: null, hosts: [], ssh: null, enrollment: null, preview: null, error: null })
})

describe('helpers', () => {
  it('splits profiles on commas and spaces, once each', () => {
    expect(parseProfiles(' base, web  web,,db ')).toEqual(['base', 'web', 'db'])
    expect(parseProfiles('   ')).toEqual([])
  })
  it('formats the extra var', () => {
    expect(tokenArg(TOKEN)).toBe(`-e keel_enroll_token=${TOKEN}`)
  })
})

describe('hosts', () => {
  it('lists hosts and the CA line, and follows pushed state', () => {
    const s = signIn()
    useHosts.getState().load('w-1')
    expect(lastCmd(s, 'ListHosts')?.cmd?.data).toEqual({ workspace: 'w-1', profile: null, drift: null })
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

  it('holds a minted token only when it answers our confirm, until dismissed', () => {
    const s = signIn()
    useHosts.getState().load('w-1')
    // A token pushed without a reply_to of ours is ignored.
    s.serverSend({
      event: {
        type: 'HostEnrollment',
        data: { id: 'e-0', workspace: 'w-1', inventory_name: 'x', profiles: [], expires_at: '', token: TOKEN },
      },
    })
    expect(useHosts.getState().enrollment).toBeNull()

    useHosts.getState().enroll('web-1', ['base'])
    expect(lastCmd(s, 'PreviewHostEnroll')?.cmd?.data).toEqual({
      workspace: 'w-1',
      inventory_name: 'web-1',
      profiles: ['base'],
    })
    answer(s, 'PreviewHostEnroll', { type: 'Preview', data: preview() })
    void useHosts.getState().confirm()
    expect(lastCmd(s, 'Confirm')?.cmd?.data).toEqual({ preview_id: 'p-1' })
    answer(
      s,
      'Confirm',
      { type: 'Confirmed', data: { preview_id: 'p-1' } },
      {
        type: 'HostEnrollment',
        data: {
          id: 'e-1',
          workspace: 'w-1',
          inventory_name: 'web-1',
          profiles: ['base'],
          expires_at: '2026-10-27T12:15:00Z',
          token: TOKEN,
        },
      },
    )
    expect(useHosts.getState().enrollment?.token).toBe(TOKEN)
    expect(useHosts.getState().preview).toBeNull()

    useHosts.getState().dismissEnrollment()
    expect(useHosts.getState().enrollment).toBeNull()
  })

  it('a revoke is previewed, and a refusal is shown', () => {
    const s = signIn()
    useHosts.getState().load('w-1')
    useHosts.getState().revoke('h-1')
    expect(lastCmd(s, 'PreviewHostRevoke')?.cmd?.data).toEqual({ host: 'h-1' })
    answer(s, 'PreviewHostRevoke', { type: 'Denied', data: { action: 'preview_host_revoke', reason: 'no_grant' } })
    expect(useHosts.getState().error).toBe('Not allowed: no_grant')
  })
})

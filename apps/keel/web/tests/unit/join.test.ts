import { describe, expect, it } from 'vitest'

import {
  acceptPrefill,
  decodeJoinClaims,
  epochToRfc3339,
  outsideAllowed,
  reasonText,
  upsertRequest,
} from '@/features/hosts/join'
import type { JoinRequestInfo } from '@/protocol/wire'

function b64url(value: unknown): string {
  return btoa(JSON.stringify(value)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

const CLAIMS = {
  iss: 'https://keel.example',
  aud: 'keel-runner',
  sub: 'ws1',
  jti: 'j-1',
  iat: 1_800_000_000,
  exp: 1_802_592_000,
  'keel.server': 'keel.example:2222',
  'keel.host_ca': 'ssh-ed25519 AAAA host-ca',
  'keel.origin': 'https://keel.example',
  'keel.rp_id': 'keel.example',
}

function token(claims: Record<string, unknown>): string {
  return `${b64url({ alg: 'EdDSA', kid: 'k1' })}.${b64url(claims)}.c2ln`
}

function req(over: Partial<JoinRequestInfo> = {}): JoinRequestInfo {
  return {
    id: 'r1',
    workspace: 'ws1',
    fingerprint: 'SHA256:abc',
    inventory_name: 'web-01',
    allowed_profiles: ['base', 'web'],
    suggested_profiles: [],
    require_signed: true,
    hostname: 'web-01.example',
    os: 'Debian GNU/Linux 13',
    arch: 'x86_64',
    runner_version: '0.1.0',
    source: '192.0.2.10',
    state: 'pending',
    reason: null,
    comment: null,
    jti: 'j-1',
    first_seen_at: '2026-11-17T10:00:00Z',
    last_seen_at: '2026-11-17T10:00:00Z',
    decided_by: null,
    decided_at: null,
    host: null,
    ...over,
  }
}

describe('decodeJoinClaims', () => {
  it('reads every claim, with the optional ones absent', () => {
    expect(decodeJoinClaims(token(CLAIMS))).toEqual({
      issuer: 'https://keel.example',
      workspace: 'ws1',
      jti: 'j-1',
      issuedAt: 1_800_000_000,
      expiresAt: 1_802_592_000,
      server: 'keel.example:2222',
      hostCa: 'ssh-ed25519 AAAA host-ca',
      origin: 'https://keel.example',
      rpId: 'keel.example',
      inventoryName: null,
      profiles: [],
    })
  })
  it('reads the suggested inventory name and profiles', () => {
    const c = decodeJoinClaims(token({ ...CLAIMS, 'keel.inventory_name': 'edge', 'keel.profiles': ['base', 3, 'web'] }))
    expect(c?.inventoryName).toBe('edge')
    expect(c?.profiles).toEqual(['base', 'web'])
  })
  it('refuses what is not a join token', () => {
    expect(decodeJoinClaims('')).toBeNull()
    expect(decodeJoinClaims('a.b')).toBeNull()
    expect(decodeJoinClaims('a.!!!.c')).toBeNull()
    expect(decodeJoinClaims(`x.${btoa('not json')}.y`)).toBeNull()
    expect(decodeJoinClaims(token({ ...CLAIMS, aud: 'keel-web' }))).toBeNull()
    const noCa = Object.fromEntries(Object.entries(CLAIMS).filter(([k]) => k !== 'keel.host_ca'))
    expect(decodeJoinClaims(token(noCa))).toBeNull()
    expect(decodeJoinClaims(token({ ...CLAIMS, exp: '1802592000' }))).toBeNull()
  })
})

describe('join helpers', () => {
  it('formats epoch seconds as RFC 3339 UTC', () => {
    expect(epochToRfc3339(1_800_000_000)).toBe('2027-01-15T08:00:00Z')
  })
  it('prefills accept with the token profiles, else base when the host allows it', () => {
    expect(acceptPrefill(req({ suggested_profiles: ['web'] }))).toEqual({ inventoryName: 'web-01', profiles: ['web'] })
    expect(acceptPrefill(req())).toEqual({ inventoryName: 'web-01', profiles: ['base'] })
    expect(acceptPrefill(req({ allowed_profiles: ['db'] })).profiles).toEqual([])
  })
  it('lists profiles outside the host allowed list', () => {
    expect(outsideAllowed(['base', 'db', 'web'], ['base', 'web'])).toEqual(['db'])
    expect(outsideAllowed([], [])).toEqual([])
  })
  it('upserts newest first, dropping requests outside the filter', () => {
    const a = req({ id: 'a', last_seen_at: '2026-11-17T10:00:00Z' })
    const b = req({ id: 'b', last_seen_at: '2026-11-17T11:00:00Z' })
    expect(upsertRequest([a], b, 'pending').map((r) => r.id)).toEqual(['b', 'a'])
    const bumped = req({ id: 'a', last_seen_at: '2026-11-17T12:00:00Z' })
    expect(upsertRequest([b, a], bumped, 'pending').map((r) => r.id)).toEqual(['a', 'b'])
    const accepted = req({ id: 'a', state: 'accepted' })
    expect(upsertRequest([b, a], accepted, 'pending').map((r) => r.id)).toEqual(['b'])
    expect(upsertRequest([b, a], accepted, null).find((r) => r.id === 'a')?.state).toBe('accepted')
  })
  it('says why a request ended, passing unknown reasons through', () => {
    expect(reasonText(null)).toBeNull()
    expect(reasonText('token_rotated')).toBe('The join token was rotated or disabled')
    expect(reasonText('something_new')).toBe('something_new')
  })
})

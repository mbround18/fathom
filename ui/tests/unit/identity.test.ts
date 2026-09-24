import { describe, expect, it } from 'vitest'

import { initialIdentityState, reduceIdentity } from '@/features/identity/state'
import { buildAuthenticationConfiguration, prefillFromIssuer } from '@/resources/oidcAuthConfig'
import { buildKubeadmFlags } from '@/resources/oidcGenerator'
import type { OidcIdentity, OidcIssuerConfig } from '@/protocol/types'

const identity: OidcIdentity = {
  issuer_id: 'rauthy',
  sub: 'u1',
  username: 'ada',
  email: null,
  groups: ['dev'],
  expires_at: 100,
  state: { type: 'SignedIn' },
}

const issuer: OidcIssuerConfig = {
  id: 'rauthy',
  issuer_url: 'https://auth.example.com/auth/v1',
  client_id: 'fathom',
  scopes: ['openid', 'groups'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

describe('identity reducer', () => {
  it('tracks progress, identity, expiry and errors', () => {
    const s = initialIdentityState()
    reduceIdentity(s, {
      type: 'OidcSignInProgress',
      data: { issuer_id: 'rauthy', stage: { type: 'Awaiting' } },
    })
    expect(s.progress.rauthy?.type).toBe('Awaiting')

    reduceIdentity(s, {
      type: 'OidcIdentityUpdated',
      data: { issuer_id: 'rauthy', identity },
    })
    expect(s.progress.rauthy).toBeUndefined()
    expect(s.identities.rauthy?.username).toBe('ada')

    reduceIdentity(s, {
      type: 'OidcSessionEnded',
      data: { issuer_id: 'rauthy', reason: 'refresh token rejected' },
    })
    expect(s.identities.rauthy?.state).toEqual({
      type: 'SignedOut',
      data: { reason: 'refresh token rejected' },
    })

    reduceIdentity(s, {
      type: 'OidcError',
      data: { issuer_id: 'rauthy', stage: 'token', message: 'boom' },
    })
    expect(s.errors.rauthy).toEqual({ stage: 'token', message: 'boom' })
    reduceIdentity(s, {
      type: 'OidcSignInProgress',
      data: { issuer_id: 'rauthy', stage: { type: 'Validating' } },
    })
    expect(s.errors.rauthy).toBeUndefined()

    reduceIdentity(s, {
      type: 'OidcIdentityUpdated',
      data: { issuer_id: 'rauthy', identity: null },
    })
    expect(s.identities.rauthy).toBeUndefined()
  })

  it('ignores unrelated events', () => {
    expect(reduceIdentity(initialIdentityState(), { type: 'BackendStarted' } as never)).toBe(false)
  })
})

describe('generator prefill from discovery', () => {
  it('prefills and renders the v1beta1 form alongside legacy flags', () => {
    const form = prefillFromIssuer(issuer)
    expect(form.issuerUrl).toBe(issuer.issuer_url)
    const yaml = buildAuthenticationConfiguration(form)
    expect(yaml).toContain('apiVersion: apiserver.config.k8s.io/v1beta1')
    expect(yaml).toContain('audienceMatchPolicy: MatchAny')
    expect(yaml).toContain('claim: preferred_username')
    expect(yaml).toContain('prefix: "oidc:"')
    expect(yaml).toContain('certificateAuthority')
    expect(buildKubeadmFlags(form)).toContain('--oidc-username-prefix=oidc:')
  })
})

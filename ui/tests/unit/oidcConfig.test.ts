/**
 * OIDC config generator unit tests (011-rbac-oidc-administration US5, T044): pure-function
 * coverage for `resources/oidcGenerator.ts`, independent of the store/React tree.
 *
 * NOTE: placed under `tests/unit/` (not `src/resources/__tests__/`) because that's the glob
 * `vitest.config.ts` actually includes (`tests/unit/**\/*.test.{ts,tsx}`) — matching this repo's
 * real test-discovery convention rather than the nominal path so `pnpm test` picks it up.
 */
import { describe, expect, it } from 'vitest'

import {
  buildKubeadmFlags,
  buildProviderInstructions,
  type OidcFormInput,
} from '@/resources/oidcGenerator'

const BASE: OidcFormInput = {
  issuerUrl: 'https://accounts.example.com',
  clientId: 'fathom-cluster',
  usernameClaim: 'email',
  groupsClaim: 'groups',
}

describe('buildKubeadmFlags', () => {
  it('produces the core flag set for a minimal input', () => {
    expect(buildKubeadmFlags(BASE)).toEqual([
      '--oidc-issuer-url=https://accounts.example.com',
      '--oidc-client-id=fathom-cluster',
      '--oidc-username-claim=email',
      '--oidc-groups-claim=groups',
    ])
  })

  it('includes --oidc-required-claim=aud=<value> for the audience case', () => {
    const input: OidcFormInput = { ...BASE, audience: 'https://kubernetes.example.com' }
    expect(buildKubeadmFlags(input)).toEqual([
      '--oidc-issuer-url=https://accounts.example.com',
      '--oidc-client-id=fathom-cluster',
      '--oidc-username-claim=email',
      '--oidc-groups-claim=groups',
      '--oidc-required-claim=aud=https://kubernetes.example.com',
    ])
  })

  it('includes username/groups prefixes when given, in the documented order', () => {
    const input: OidcFormInput = {
      ...BASE,
      usernamePrefix: 'oidc:',
      groupsPrefix: 'oidc:',
      audience: 'my-audience',
    }
    expect(buildKubeadmFlags(input)).toEqual([
      '--oidc-issuer-url=https://accounts.example.com',
      '--oidc-client-id=fathom-cluster',
      '--oidc-username-claim=email',
      '--oidc-username-prefix=oidc:',
      '--oidc-groups-claim=groups',
      '--oidc-groups-prefix=oidc:',
      '--oidc-required-claim=aud=my-audience',
    ])
  })

  it('never emits a flag for extraScopes (not a kube-apiserver concept)', () => {
    const input: OidcFormInput = { ...BASE, extraScopes: 'openid, profile, email' }
    const flags = buildKubeadmFlags(input)
    expect(flags.some((f) => f.includes('scope'))).toBe(false)
  })
})

describe('buildProviderInstructions', () => {
  it('returns entries for EKS, GKE and AKS, each with non-empty instruction text', () => {
    const blocks = buildProviderInstructions(BASE)
    const providers = blocks.map((b) => b.provider)
    expect(providers).toEqual(expect.arrayContaining(['EKS', 'GKE', 'AKS']))
    expect(blocks).toHaveLength(3)
    for (const block of blocks) {
      expect(block.instructions.length).toBeGreaterThan(0)
      for (const line of block.instructions) {
        expect(line.trim().length).toBeGreaterThan(0)
      }
    }
  })

  it('mentions the requested scopes in instruction text when extraScopes is given', () => {
    const input: OidcFormInput = { ...BASE, extraScopes: 'openid, profile' }
    const blocks = buildProviderInstructions(input)
    const allText = blocks.flatMap((b) => b.instructions).join(' ')
    expect(allText).toContain('openid, profile')
  })
})

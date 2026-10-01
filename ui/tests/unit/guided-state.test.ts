/** 045 T015: every transition of the guided OIDC path's pure step machine (data-model.md § UI draft). */
import { describe, expect, it } from 'vitest'

import {
  canAdvance,
  canSave,
  EMPTY_ISSUER_CONFIG,
  EMPTY_ROLE_MAP,
  guidedReducer,
  initialGuidedDraft,
  issuerMismatch,
  isFrozen,
  stepsOf,
  unverified,
  type GuidedAction,
  type GuidedDraft,
  type GuidedStep,
} from '@/features/identity/guided-state'
import type {
  ApiServerAuthComparison,
  Discovery,
  DraftSignInResult,
  OidcIssuerConfig,
} from '@/protocol/types'

const ISSUER_URL = 'https://idp.example.com/realms/fathom'

const CONFIG: OidcIssuerConfig = {
  ...EMPTY_ISSUER_CONFIG,
  id: 'idp',
  issuer_url: ISSUER_URL,
  client_id: 'fathom',
}

const DISCOVERY: Discovery = {
  issuer: ISSUER_URL,
  authorization_endpoint: `${ISSUER_URL}/protocol/openid-connect/auth`,
  token_endpoint: `${ISSUER_URL}/protocol/openid-connect/token`,
  device_authorization_endpoint: null,
  jwks_uri: `${ISSUER_URL}/protocol/openid-connect/certs`,
  end_session_endpoint: null,
  scopes_supported: ['openid', 'groups'],
  claims_supported: ['sub', 'preferred_username', 'groups'],
}

const SIGN_IN: DraftSignInResult = {
  username: 'ada',
  groups: ['platform'],
  claims: [{ name: 'preferred_username', value: 'ada' }],
  expires_at: 1790000000,
}

const COMPARISON: ApiServerAuthComparison = {
  read: {
    style: 'Legacy',
    issuer: ISSUER_URL,
    client_id: 'fathom',
    username_claim: 'preferred_username',
    groups_claim: 'groups',
    username_prefix: 'oidc:',
    groups_prefix: 'oidc:',
    context: 'kind-fathom-dev',
    checked: true,
    reason: null,
  },
  differences: [],
  writable: true,
  not_writable_reason: null,
}

/** A draft sitting on `step` with everything the steps before it would have gathered. */
function at(step: GuidedStep, over: Partial<GuidedDraft> = {}): GuidedDraft {
  return {
    ...initialGuidedDraft(CONFIG, EMPTY_ROLE_MAP),
    step,
    discovery: DISCOVERY,
    ...over,
  }
}

const run = (d: GuidedDraft, ...actions: GuidedAction[]): GuidedDraft =>
  actions.reduce(guidedReducer, d)

describe('initialGuidedDraft', () => {
  it('starts empty, unsaved and on the first step', () => {
    const d = initialGuidedDraft()
    expect(d).toEqual({
      step: 'issuer-url',
      config: EMPTY_ISSUER_CONFIG,
      roleMap: EMPTY_ROLE_MAP,
      origin: 'Saved',
      saved: false,
    })
    expect(isFrozen(d)).toBe(false)
  })

  it('pre-fills from an existing issuer for "Review…"', () => {
    const d = initialGuidedDraft(CONFIG, { ...EMPTY_ROLE_MAP, admins_group: 'platform' })
    expect(d.config).toBe(CONFIG)
    expect(d.roleMap.admins_group).toBe('platform')
    expect(d.step).toBe('issuer-url')
  })
})

describe('stepsOf', () => {
  it('withholds the post-save steps until the save happened', () => {
    expect(stepsOf(at('review-save'))).toEqual([
      'issuer-url',
      'discovery',
      'test-sign-in',
      'claims',
      'cluster-compare',
      'fathom-roles',
      'review-save',
    ])
    expect(stepsOf(at('review-save', { saved: true }))).toEqual([
      'issuer-url',
      'discovery',
      'test-sign-in',
      'claims',
      'cluster-compare',
      'fathom-roles',
      'review-save',
      'bindings',
      'apiserver-write',
    ])
  })

  it('skips review-save for a seeded issuer, keeping every review step', () => {
    const seeded = at('claims', { origin: 'SeededFromEnvironment' })
    expect(isFrozen(seeded)).toBe(true)
    expect(stepsOf(seeded)).toEqual([
      'issuer-url',
      'discovery',
      'test-sign-in',
      'claims',
      'cluster-compare',
      'fathom-roles',
    ])
  })
})

describe('next / back', () => {
  it('walks the whole path forwards and back again', () => {
    let d = at('issuer-url')
    const visited: GuidedStep[] = [d.step]
    for (let i = 0; i < 10; i += 1) {
      const n = guidedReducer(d, { type: 'next' })
      if (n === d) break
      d = n
      visited.push(d.step)
    }
    expect(visited).toEqual([
      'issuer-url',
      'discovery',
      'test-sign-in',
      'claims',
      'cluster-compare',
      'fathom-roles',
      'review-save',
    ])

    // FR-025: every step is reachable backwards without losing the draft.
    for (let i = 0; i < 10; i += 1) d = guidedReducer(d, { type: 'back' })
    expect(d.step).toBe('issuer-url')
    expect(d.config).toEqual(CONFIG)
    expect(d.discovery).toEqual(DISCOVERY)
  })

  it('refuses to leave the URL step until both strings are entered', () => {
    const empty = initialGuidedDraft()
    expect(canAdvance(empty)).toBe(false)
    expect(guidedReducer(empty, { type: 'next' })).toBe(empty)

    const urlOnly = guidedReducer(empty, {
      type: 'edit-config',
      patch: { issuer_url: ISSUER_URL },
    })
    expect(canAdvance(urlOnly)).toBe(false)

    const both = guidedReducer(urlOnly, { type: 'edit-config', patch: { client_id: 'fathom' } })
    expect(canAdvance(both)).toBe(true)
    expect(guidedReducer(both, { type: 'next' }).step).toBe('discovery')
  })

  it('refuses to leave the discovery step before the provider has answered', () => {
    const unprobed = at('discovery', { discovery: undefined })
    expect(canAdvance(unprobed)).toBe(false)
    expect(guidedReducer(unprobed, { type: 'next' })).toBe(unprobed)
  })

  it('stops on the discovery step when the declared issuer disagrees (FR-002)', () => {
    const d = at('discovery', {
      discovery: { ...DISCOVERY, issuer: 'https://idp.example.com/realms/other' },
    })
    expect(issuerMismatch(d)).toEqual({
      typed: ISSUER_URL,
      declared: 'https://idp.example.com/realms/other',
    })
    expect(canAdvance(d)).toBe(false)
    expect(guidedReducer(d, { type: 'next' })).toBe(d)
    // The refusal is forwards only; the operator fixes the URL on the step before.
    expect(guidedReducer(d, { type: 'back' }).step).toBe('issuer-url')
  })

  it('is a no-op at both ends of the path', () => {
    const first = at('issuer-url')
    expect(guidedReducer(first, { type: 'back' })).toBe(first)
    const last = at('apiserver-write', { saved: true })
    expect(guidedReducer(last, { type: 'next' })).toBe(last)
  })
})

describe('goto', () => {
  it('moves to any reachable step, and back past a blocking one', () => {
    const d = at('issuer-url')
    expect(guidedReducer(d, { type: 'goto', step: 'cluster-compare' }).step).toBe('cluster-compare')
    expect(guidedReducer(at('fathom-roles'), { type: 'goto', step: 'claims' }).step).toBe('claims')
  })

  it('refuses a forward jump out of a blocking step', () => {
    const d = at('discovery', { discovery: undefined })
    expect(guidedReducer(d, { type: 'goto', step: 'review-save' })).toBe(d)
    expect(guidedReducer(d, { type: 'goto', step: 'issuer-url' }).step).toBe('issuer-url')
  })

  it('refuses the post-save steps before a save (FR-027)', () => {
    const d = at('review-save')
    expect(guidedReducer(d, { type: 'goto', step: 'bindings' })).toBe(d)
    expect(guidedReducer(d, { type: 'goto', step: 'apiserver-write' })).toBe(d)

    const after = guidedReducer(d, { type: 'saved' })
    expect(after.saved).toBe(true)
    expect(guidedReducer(after, { type: 'goto', step: 'bindings' }).step).toBe('bindings')
    expect(guidedReducer(after, { type: 'next' }).step).toBe('bindings')
  })

  it('refuses review-save for a seeded issuer', () => {
    const seeded = at('claims', { origin: 'SeededFromEnvironment' })
    expect(guidedReducer(seeded, { type: 'goto', step: 'review-save' })).toBe(seeded)
    // `next` from the last reachable step is a no-op rather than a jump into it.
    expect(
      guidedReducer(at('fathom-roles', { origin: 'SeededFromEnvironment' }), { type: 'next' }),
    ).toMatchObject({ step: 'fathom-roles' })
  })
})

describe('edit-config', () => {
  it('discards the evidence gathered from the previous URL', () => {
    const d = at('cluster-compare', { signIn: SIGN_IN, comparison: COMPARISON })
    const moved = guidedReducer(d, {
      type: 'edit-config',
      patch: { issuer_url: 'https://other.example.com' },
    })
    expect(moved.discovery).toBeUndefined()
    expect(moved.signIn).toBeUndefined()
    expect(moved.comparison).toBeUndefined()
    expect(moved.step).toBe('cluster-compare')
  })

  it('keeps the evidence when another field changes', () => {
    const d = at('claims', { signIn: SIGN_IN, comparison: COMPARISON })
    const next = guidedReducer(d, { type: 'edit-config', patch: { groups_prefix: 'k8s:' } })
    expect(next.config.groups_prefix).toBe('k8s:')
    expect(next.discovery).toEqual(DISCOVERY)
    expect(next.signIn).toEqual(SIGN_IN)
  })

  it('freezes the config of a seeded issuer (FR-032)', () => {
    const seeded = at('claims', { origin: 'SeededFromEnvironment' })
    expect(guidedReducer(seeded, { type: 'edit-config', patch: { client_id: 'other' } })).toBe(
      seeded,
    )
  })
})

describe('backend replies', () => {
  it('shows the discovery document and advances off the URL step', () => {
    const d = guidedReducer(at('issuer-url', { discovery: undefined }), {
      type: 'discovered',
      issuerUrl: ISSUER_URL,
      discovery: DISCOVERY,
    })
    expect(d.step).toBe('discovery')
    expect(d.discovery).toEqual(DISCOVERY)
  })

  it('does not move the operator off a later step they already reached', () => {
    const d = guidedReducer(at('claims', { discovery: undefined }), {
      type: 'discovered',
      issuerUrl: ISSUER_URL,
      discovery: DISCOVERY,
    })
    expect(d.step).toBe('claims')
    expect(d.discovery).toEqual(DISCOVERY)
  })

  it('drops a discovery reply for a URL the draft no longer holds', () => {
    const d = at('issuer-url', { discovery: undefined })
    expect(
      guidedReducer(d, {
        type: 'discovered',
        issuerUrl: 'https://stale.example.com',
        discovery: DISCOVERY,
      }),
    ).toBe(d)
  })

  it('records the test sign-in and the cluster comparison without moving', () => {
    const signedIn = guidedReducer(at('test-sign-in'), { type: 'signed-in', result: SIGN_IN })
    expect(signedIn.signIn).toEqual(SIGN_IN)
    expect(signedIn.step).toBe('test-sign-in')

    const compared = guidedReducer(at('cluster-compare'), {
      type: 'compared',
      comparison: COMPARISON,
    })
    expect(compared.comparison).toEqual(COMPARISON)
    expect(compared.step).toBe('cluster-compare')
  })
})

describe('canSave', () => {
  it('is available from the first step once the stored fields are present (FR-015)', () => {
    expect(canSave(initialGuidedDraft())).toBe(false)
    expect(canSave(at('issuer-url'))).toBe(true)
    expect(canSave(at('issuer-url', { config: { ...CONFIG, id: '  ' } }))).toBe(false)
  })

  it('is never available for a seeded issuer (FR-032)', () => {
    expect(canSave(at('claims', { origin: 'SeededFromEnvironment' }))).toBe(false)
  })
})

describe('open', () => {
  it('resets the whole draft, including a save that already happened', () => {
    const d = run(at('apiserver-write', { saved: true, signIn: SIGN_IN }), {
      type: 'open',
      config: EMPTY_ISSUER_CONFIG,
      roleMap: EMPTY_ROLE_MAP,
      origin: 'Saved',
    })
    expect(d).toEqual(initialGuidedDraft())
  })
})

/**
 * T041/FR-027: the Fathom role slots and the cluster bindings are two separate steps, and refusing
 * the second one is not a refusal of the first. "Refusing" is leaving — the bindings step offers a
 * handoff and nothing else, so walking away from it is the whole refusal, and the draft it walks
 * away from must still be the saved issuer.
 */
describe('the Fathom slots and the cluster bindings are separate (FR-027)', () => {
  it('are two distinct steps, with the save between them', () => {
    const steps = stepsOf(at('fathom-roles', { saved: true }))
    expect(steps).toContain('fathom-roles')
    expect(steps).toContain('bindings')
    expect(steps.indexOf('fathom-roles')).toBeLessThan(steps.indexOf('review-save'))
    expect(steps.indexOf('review-save')).toBeLessThan(steps.indexOf('bindings'))
  })

  it('offers no bindings step before the save, because a binding needs a saved issuer id', () => {
    expect(stepsOf(at('fathom-roles'))).not.toContain('bindings')
    // And the step cannot be jumped to either: `goto` only visits steps this draft has.
    expect(run(at('fathom-roles'), { type: 'goto', step: 'bindings' }).step).toBe('fathom-roles')
  })

  it('lets the Fathom slots be filled and left without touching the bindings', () => {
    const d = run(at('fathom-roles', { signIn: SIGN_IN }), {
      type: 'edit-role-map',
      patch: { admins_group: 'platform' },
    })
    expect(d.roleMap.admins_group).toBe('platform')
    // Filling a Fathom slot is not a cluster grant, and the machine models no such coupling.
    expect(d.saved).toBe(false)
    expect(canAdvance(d)).toBe(true)
  })

  it('leaves the saved issuer intact when the bindings step is refused', () => {
    const saved = run(at('review-save'), { type: 'saved' })
    expect(saved.saved).toBe(true)
    const refused = run(saved, { type: 'goto', step: 'bindings' }, { type: 'back' })
    // Back out of the bindings and the issuer is still saved, with its config and role map.
    expect(refused.saved).toBe(true)
    expect(refused.config).toEqual(saved.config)
    expect(refused.roleMap).toEqual(saved.roleMap)
    // Nothing about refusing the bindings removes the steps the save unlocked.
    expect(stepsOf(refused)).toContain('bindings')
  })
})

describe('unverified', () => {
  it('names each verification that was skipped (FR-014, FR-015)', () => {
    const bare = { ...initialGuidedDraft(CONFIG), discovery: undefined }
    const missing = unverified(bare).join(' ')
    expect(missing).toContain('discovery document')
    expect(missing).toContain('test sign-in')
    expect(missing).toContain('API server configuration')
  })

  it('is empty once all three were performed and the cluster agrees', () => {
    expect(unverified(at('review-save', { signIn: SIGN_IN, comparison: COMPARISON }))).toEqual([])
  })

  it('counts an unreadable API server as unverified, with its verbatim reason (FR-010)', () => {
    const comparison: ApiServerAuthComparison = {
      ...COMPARISON,
      read: { ...COMPARISON.read, checked: false, reason: 'pods "kube-apiserver" is forbidden' },
    }
    expect(unverified(at('review-save', { signIn: SIGN_IN, comparison })).join(' ')).toContain(
      'pods "kube-apiserver" is forbidden',
    )
  })

  it('counts a disagreeing API server as unverified, naming the fields', () => {
    const comparison: ApiServerAuthComparison = {
      ...COMPARISON,
      differences: [{ field: 'groups_prefix', cluster: 'k8s:', draft: 'oidc:' }],
    }
    expect(unverified(at('review-save', { signIn: SIGN_IN, comparison })).join(' ')).toContain(
      'groups_prefix',
    )
  })

  it('does not ask a seeded issuer for a discovery document it cannot fetch (FR-032)', () => {
    const seeded = at('claims', { origin: 'SeededFromEnvironment', discovery: undefined })
    expect(unverified(seeded).join(' ')).not.toContain('discovery document')
  })
})

describe('a seeded draft can be read end to end (FR-032)', () => {
  it('advances past the discovery step with no document, which hosted cannot fetch', () => {
    const seeded = at('discovery', { origin: 'SeededFromEnvironment', discovery: undefined })
    expect(canAdvance(seeded)).toBe(true)
    expect(run(seeded, { type: 'next' }).step).toBe('test-sign-in')
  })

  it('still refuses to leave the discovery step of a saved draft with no document', () => {
    expect(canAdvance(at('discovery', { discovery: undefined }))).toBe(false)
  })
})

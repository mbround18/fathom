/**
 * 045: which step the guided OIDC path is on and what its draft holds (data-model.md § UI draft).
 * Pure, so every transition is unit tested.
 *
 * Nothing here is persisted. The draft *is* FR-007's "writes nothing before the save" guarantee:
 * closing the Sheet discards this object, so abandoning the path cannot leave a half-configured
 * issuer behind — by construction rather than by cleanup.
 */
import type {
  ApiServerAuthComparison,
  Discovery,
  DraftSignInResult,
  FathomRoleMap,
  IssuerOrigin,
  OidcIssuerConfig,
} from '@/protocol/types'

export type GuidedStep =
  | 'issuer-url'
  | 'discovery'
  // After the sign-in, not before it: FR-005 chooses the username and groups claims from the
  // claims a real token carried. With the sign-in skipped, the step falls back to the document's
  // advertised `claims_supported`, labelled as advertised rather than observed.
  | 'test-sign-in'
  | 'claims'
  | 'cluster-compare'
  | 'fathom-roles'
  | 'review-save'
  | 'bindings'
  | 'apiserver-write'

export interface GuidedDraft {
  step: GuidedStep
  config: OidcIssuerConfig
  /** What the provider's discovery document declared. Evidence, never stored. */
  discovery?: Discovery
  /** What one test sign-in proved. Evidence, never stored, carries no token material. */
  signIn?: DraftSignInResult
  /** The API server's own authentication configuration beside this draft. */
  comparison?: ApiServerAuthComparison
  roleMap: FathomRoleMap
  origin: IssuerOrigin
  /**
   * The save has happened. Not a persisted field — nothing here is — but the boundary the last
   * two steps sit behind: 042's `RoleAssignmentDraft` needs a saved `issuer_id`, so neither the
   * bindings handoff nor the API server write can exist before it.
   */
  saved: boolean
}

/** A new issuer's starting point: the same defaults the flat form offered. */
export const EMPTY_ISSUER_CONFIG: OidcIssuerConfig = {
  id: '',
  issuer_url: '',
  client_id: '',
  scopes: ['openid', 'profile', 'email', 'groups'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

export const EMPTY_ROLE_MAP: FathomRoleMap = {
  viewers_group: '',
  editors_group: '',
  admins_group: '',
}

/** Declaration order is path order; `stepsOf` is the only thing that decides which ones exist. */
const STEP_ORDER: readonly GuidedStep[] = [
  'issuer-url',
  'discovery',
  'test-sign-in',
  'claims',
  'cluster-compare',
  'fathom-roles',
  'review-save',
  'bindings',
  'apiserver-write',
]

export type GuidedAction =
  /** Open the path: empty for "Add issuer…", pre-filled for an existing issuer's "Review…". */
  | { type: 'open'; config: OidcIssuerConfig; roleMap: FathomRoleMap; origin: IssuerOrigin }
  | { type: 'goto'; step: GuidedStep }
  | { type: 'next' }
  | { type: 'back' }
  | { type: 'edit-config'; patch: Partial<OidcIssuerConfig> }
  | { type: 'edit-role-map'; patch: Partial<FathomRoleMap> }
  /** `OidcDraftDiscovered`; `issuerUrl` is echoed back so a stale reply is ignorable. */
  | { type: 'discovered'; issuerUrl: string; discovery: Discovery }
  /** `OidcDraftSignInSucceeded`. */
  | { type: 'signed-in'; result: DraftSignInResult }
  /** The `ApiServerAuthRead` reply for the context the path is comparing against. */
  | { type: 'compared'; comparison: ApiServerAuthComparison }
  /** `OidcSaveDraft` succeeded: the boundary between "nothing written" and "writes offered". */
  | { type: 'saved' }

export function initialGuidedDraft(
  config: OidcIssuerConfig = EMPTY_ISSUER_CONFIG,
  roleMap: FathomRoleMap = EMPTY_ROLE_MAP,
  origin: IssuerOrigin = 'Saved',
): GuidedDraft {
  return { step: 'issuer-url', config, roleMap, origin, saved: false }
}

/** A seeded issuer was validated by the running server before it bound a socket (FR-032). */
export function isFrozen(d: GuidedDraft): boolean {
  return d.origin === 'SeededFromEnvironment'
}

/**
 * The steps this draft can visit. A seeded issuer has no `review-save` — there is nothing to
 * save, and the place it is changed is a mounted Secret, not a Fathom screen (FR-032).
 */
export function stepsOf(d: GuidedDraft): readonly GuidedStep[] {
  return STEP_ORDER.filter((step) => {
    if (step === 'review-save') return !isFrozen(d)
    if (step === 'bindings' || step === 'apiserver-write') return d.saved
    return true
  })
}

/**
 * The provider's declared `issuer` against the URL typed. A disagreement is not cosmetic: tokens
 * carry the declared value in `iss`, so every later validation would fail (FR-002).
 */
export function issuerMismatch(d: GuidedDraft): { typed: string; declared: string } | undefined {
  const declared = d.discovery?.issuer
  if (declared === undefined) return undefined
  const typed = d.config.issuer_url.trim()
  return declared === typed ? undefined : { typed, declared }
}

/** Whether the current step lets the path move forward. Moving *back* is never blocked (FR-025). */
export function canAdvance(d: GuidedDraft): boolean {
  switch (d.step) {
    case 'issuer-url':
      return d.config.issuer_url.trim() !== '' && d.config.client_id.trim() !== ''
    case 'discovery':
      // The one refusal in the path: carrying an issuer mismatch forward would configure an
      // issuer whose every token is rejected, which is what FR-002 exists to stop.
      return d.discovery !== undefined && issuerMismatch(d) === undefined
    default:
      return true
  }
}

/**
 * "Save without testing", offered at every step (FR-015): the three strings the stored issuer
 * cannot be without. Verification is optional; a seeded issuer is not saveable at all.
 */
export function canSave(d: GuidedDraft): boolean {
  if (isFrozen(d)) return false
  const { id, issuer_url, client_id } = d.config
  return id.trim() !== '' && issuer_url.trim() !== '' && client_id.trim() !== ''
}

export function guidedReducer(d: GuidedDraft, action: GuidedAction): GuidedDraft {
  switch (action.type) {
    case 'open':
      return initialGuidedDraft(action.config, action.roleMap, action.origin)

    case 'goto': {
      const steps = stepsOf(d)
      if (!steps.includes(action.step)) return d
      // Forward past a blocking step is the same refusal `next` makes; backward is always free.
      const forward = steps.indexOf(action.step) > steps.indexOf(d.step)
      if (forward && !canAdvance(d)) return d
      return { ...d, step: action.step }
    }

    case 'next': {
      const steps = stepsOf(d)
      const i = steps.indexOf(d.step)
      const next = i < 0 ? undefined : steps[i + 1]
      if (next === undefined || !canAdvance(d)) return d
      return { ...d, step: next }
    }

    case 'back': {
      const steps = stepsOf(d)
      const i = steps.indexOf(d.step)
      const prev = i <= 0 ? undefined : steps[i - 1]
      if (prev === undefined) return d
      return { ...d, step: prev }
    }

    case 'edit-config': {
      if (isFrozen(d)) return d
      const config = { ...d.config, ...action.patch }
      // Everything already gathered belongs to the URL it was gathered from, so re-pointing the
      // draft discards it rather than showing one provider's document beside another's.
      if (config.issuer_url !== d.config.issuer_url) {
        return { ...d, config, discovery: undefined, signIn: undefined, comparison: undefined }
      }
      return { ...d, config }
    }

    case 'edit-role-map':
      return { ...d, roleMap: { ...d.roleMap, ...action.patch } }

    case 'discovered': {
      // The reply may land after the operator edited the field; it is then about a URL this
      // draft no longer holds, so it is dropped rather than rendered against the new one.
      if (action.issuerUrl !== d.config.issuer_url) return d
      const next = { ...d, discovery: action.discovery }
      return d.step === 'issuer-url' ? { ...next, step: 'discovery' } : next
    }

    case 'signed-in':
      return { ...d, signIn: action.result }

    case 'compared':
      return { ...d, comparison: action.comparison }

    case 'saved':
      return { ...d, saved: true }
  }
}

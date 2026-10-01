/**
 * 033 US1: pure identity state, fed by the `Oidc*` system events (`store/handlers/system.ts`).
 * Tokens never reach the UI: only the non-secret `OidcIdentity` and sign-in progress do.
 */
import type {
  ApiServerAuthComparison,
  Discovery,
  DraftSignInResult,
  IssuerOrigin,
  OidcConfig,
  OidcIdentity,
  OidcSignInStage,
  SystemEvent,
} from '@/protocol/types'

export interface IdentityState {
  config: OidcConfig
  /** Signed-in identity per issuer id. */
  identities: Record<string, OidcIdentity>
  /** In-flight sign-in stage per issuer id (absent when idle). */
  progress: Record<string, OidcSignInStage>
  /** Last verbatim error per issuer id, cleared when a new sign-in starts or succeeds. */
  errors: Record<string, { stage: string; message: string }>
  /**
   * 045: the last draft probe's reply. Evidence for the guided path, never persisted; the URL is
   * echoed back so the path's own reducer can drop a reply for a URL it no longer holds.
   */
  draftDiscovery?: { issuer_url: string; discovery: Discovery }
  /** 045: what the last draft sign-in proved. Carries no token material (FR-004). */
  draftSignIn?: DraftSignInResult
  /**
   * 045: the API server's authentication configuration beside the draft, per kubeconfig context —
   * the comparison belongs to the context it was read from, so it cannot be a single slot (FR-026).
   */
  apiServerAuth: Record<string, ApiServerAuthComparison>
  /**
   * 045: the context the last `ApiServerAuthRead` was read from. The guided path renders the
   * comparison it was given even when the server read a context the UI does not call active —
   * hosted reads its own in-cluster context — and the step names that context and marks it
   * stale rather than waiting for a read that will never be keyed the way it expects (FR-026).
   */
  apiServerAuthLast?: string
  /**
   * 045: issuer origins the server declared (FR-031). Only hosted emits them; `Saved` is the
   * absence of an entry, so an empty map is the desktop case rather than a missing read.
   */
  origins: Record<string, { origin: IssuerOrigin; source: string }>
}

export function initialIdentityState(): IdentityState {
  return {
    config: { issuers: [], contexts: [], role_map: {} as OidcConfig['role_map'] },
    identities: {},
    progress: {},
    errors: {},
    apiServerAuth: {},
    origins: {},
  }
}

/** Applies one `Oidc*` event; returns false when the event is not an identity event. */
export function reduceIdentity(s: IdentityState, event: SystemEvent): boolean {
  switch (event.type) {
    case 'OidcConfigLoaded': {
      s.config = event.data.config
      s.identities = Object.fromEntries(event.data.identities.map((i) => [i.issuer_id, i]))
      return true
    }
    case 'OidcSignInProgress': {
      const { issuer_id, stage } = event.data
      if (stage.type === 'Done') Reflect.deleteProperty(s.progress, issuer_id)
      else s.progress[issuer_id] = stage
      Reflect.deleteProperty(s.errors, issuer_id)
      return true
    }
    case 'OidcIdentityUpdated': {
      const { issuer_id, identity } = event.data
      if (identity) s.identities[issuer_id] = identity
      else Reflect.deleteProperty(s.identities, issuer_id)
      Reflect.deleteProperty(s.progress, issuer_id)
      return true
    }
    case 'OidcSessionEnded': {
      const { issuer_id, reason } = event.data
      const cur = s.identities[issuer_id]
      if (cur) {
        s.identities[issuer_id] = { ...cur, state: { type: 'SignedOut', data: { reason } } }
      }
      Reflect.deleteProperty(s.progress, issuer_id)
      return true
    }
    // 045: the three replies the guided path's steps render. None of them is configuration — a
    // discovery document and a token's claims are evidence, and nothing here is persisted.
    case 'OidcDraftDiscovered': {
      s.draftDiscovery = event.data
      return true
    }
    case 'OidcDraftSignInSucceeded': {
      s.draftSignIn = event.data.result
      return true
    }
    case 'OidcSeedOrigin': {
      const { issuer_id, origin, source } = event.data
      s.origins[issuer_id] = { origin, source }
      return true
    }
    case 'OidcError': {
      const { issuer_id, stage, message } = event.data
      s.errors[issuer_id] = { stage, message }
      Reflect.deleteProperty(s.progress, issuer_id)
      return true
    }
    default:
      return false
  }
}

/**
 * 045: the `ApiServerAuthRead` read event. Keyed by the context it was read from, not by the
 * generic `details` keying — an `ApiServerAuthComparison` has no `name` field, and a comparison
 * read from one context says nothing about another (FR-026).
 */
export function reduceApiServerAuth(s: IdentityState, read: ApiServerAuthComparison): void {
  s.apiServerAuth[read.read.context] = read
  s.apiServerAuthLast = read.read.context
}

export function isSignedIn(i: OidcIdentity | undefined): boolean {
  return !!i && i.state.type !== 'SignedOut'
}

/**
 * Whether the operator may administer Fathom-level settings: any signed-in user while no
 * admins group is configured (bootstrap), otherwise only members of it. Mirrors the backend's
 * `require_role(.., Admin)`, which is the real gate.
 */
export function isFathomAdmin(
  me: OidcIdentity | undefined,
  roleMap: { admins_group: string },
): boolean {
  return !roleMap.admins_group || (!!me && me.groups.includes(roleMap.admins_group))
}

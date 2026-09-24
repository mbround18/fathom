/**
 * 033 US1: pure identity state, fed by the `Oidc*` system events (`store/handlers/system.ts`).
 * Tokens never reach the UI: only the non-secret `OidcIdentity` and sign-in progress do.
 */
import type { OidcConfig, OidcIdentity, OidcSignInStage, SystemEvent } from '@/protocol/types'

export interface IdentityState {
  config: OidcConfig
  /** Signed-in identity per issuer id. */
  identities: Record<string, OidcIdentity>
  /** In-flight sign-in stage per issuer id (absent when idle). */
  progress: Record<string, OidcSignInStage>
  /** Last verbatim error per issuer id, cleared when a new sign-in starts or succeeds. */
  errors: Record<string, { stage: string; message: string }>
}

export function initialIdentityState(): IdentityState {
  return {
    config: { issuers: [], contexts: [], role_map: {} as OidcConfig['role_map'] },
    identities: {},
    progress: {},
    errors: {},
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

export function isSignedIn(i: OidcIdentity | undefined): boolean {
  return !!i && i.state.type !== 'SignedOut'
}

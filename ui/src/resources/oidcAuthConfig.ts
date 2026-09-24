/**
 * 033 US2: the structured `AuthenticationConfiguration` (apiserver.config.k8s.io/v1beta1, passed
 * via `--authentication-config`, Kubernetes 1.30+) and the prefill of the 011 generator from a
 * configured issuer. Preferred over the legacy flags, which stay available for older clusters.
 */
import type { OidcFormInput } from './oidcGenerator'

/** Audiences use `MatchAny`; claim mappings carry the prefixes. */
export function buildAuthenticationConfiguration(input: OidcFormInput): string {
  const audiences = (input.audience || input.clientId)
    .split(',')
    .map((a) => a.trim())
    .filter(Boolean)
  const lines = [
    'apiVersion: apiserver.config.k8s.io/v1beta1',
    'kind: AuthenticationConfiguration',
    'jwt:',
    '- issuer:',
    `    url: ${input.issuerUrl}`,
    '    audiences:',
    ...audiences.map((a) => `    - ${a}`),
    '    audienceMatchPolicy: MatchAny',
    '    # certificateAuthority: |',
    '    #   <PEM bundle, only when the issuer uses a private CA>',
    '  claimMappings:',
    '    username:',
    `      claim: ${input.usernameClaim}`,
    `      prefix: ${JSON.stringify(input.usernamePrefix ?? '')}`,
    '    groups:',
    `      claim: ${input.groupsClaim}`,
    `      prefix: ${JSON.stringify(input.groupsPrefix ?? '')}`,
  ]
  return lines.join('\n')
}

/** Minimal shape of a configured Fathom issuer used to prefill the generator. */
export interface IssuerPrefillSource {
  issuer_url: string
  client_id: string
  scopes: string[]
  username_claim: string
  groups_claim: string
  username_prefix: string | null
  groups_prefix: string | null
}

export function prefillFromIssuer(issuer: IssuerPrefillSource): OidcFormInput {
  return {
    issuerUrl: issuer.issuer_url,
    clientId: issuer.client_id,
    usernameClaim: issuer.username_claim,
    groupsClaim: issuer.groups_claim,
    usernamePrefix: issuer.username_prefix ?? '',
    groupsPrefix: issuer.groups_prefix ?? '',
    audience: issuer.client_id,
    extraScopes: issuer.scopes.join(', '),
  }
}

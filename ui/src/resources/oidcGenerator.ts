/**
 * 011-rbac-oidc-administration (US5, T043-T044): pure, framework-free config generation for the
 * OIDC-config generator panel (`oidcConfig.tsx`). Fathom never applies any of this to a cluster —
 * these functions only produce text for the operator to copy/export and apply themselves.
 *
 * Flag confidence (see also the final report for this task): `--oidc-issuer-url`,
 * `--oidc-client-id`, `--oidc-username-claim`, `--oidc-groups-claim`, `--oidc-username-prefix`
 * and `--oidc-groups-prefix` are long-standing, stable `kube-apiserver` flags unchanged across
 * many Kubernetes releases — high confidence. Audience validation has no dedicated
 * `--oidc-audience`-style flag; the current upstream mechanism is the generic
 * `--oidc-required-claim=<claim>=<value>` flag (repeatable per required claim), used here as
 * `--oidc-required-claim=aud=<audience>` — also high confidence, but double-check against the
 * Kubernetes version in use since `--oidc-required-claim` is newer than the core four flags.
 * `extraScopes` is deliberately NOT turned into a `kube-apiserver` flag: the API server's OIDC
 * authenticator only ever validates an ID token it's handed, it never performs the browser/device
 * OIDC login flow itself, so "scopes" are a property of the identity provider's client
 * registration and of whatever tool the operator uses to obtain a token (e.g. `kubelogin`,
 * `kubectl oidc-login`), not of `kube-apiserver`. We surface `extraScopes` only in the
 * human-readable instructions, never as a flag.
 */

/** Shape of the OIDC generator form (US5). All fields are plain strings from form inputs. */
export interface OidcFormInput {
  /** The OIDC issuer URL, e.g. `https://accounts.example.com`. */
  issuerUrl: string
  /** The OIDC client ID registered with the issuer for this cluster. */
  clientId: string
  /** JWT claim to use as the Kubernetes username (commonly `sub` or `email`). */
  usernameClaim: string
  /** JWT claim to use as the Kubernetes groups (commonly `groups`). */
  groupsClaim: string
  /** Optional prefix prepended to the username claim value (`--oidc-username-prefix`). */
  usernamePrefix?: string | undefined
  /** Optional prefix prepended to each group claim value (`--oidc-groups-prefix`). */
  groupsPrefix?: string | undefined
  /**
   * Optional expected audience. Rendered as `--oidc-required-claim=aud=<audience>`, not a
   * dedicated audience flag (there isn't one — see the module comment).
   */
  audience?: string | undefined
  /**
   * Optional comma-separated extra OAuth2/OIDC scopes to request when obtaining a token
   * (informational only — not a `kube-apiserver` flag; see the module comment).
   */
  extraScopes?: string | undefined
}

/**
 * Produce the exact `kube-apiserver` OIDC flag set for `input`. Order is stable: issuer, client
 * id, username claim, username prefix (if any), groups claim, groups prefix (if any), required
 * audience claim (if any).
 */
export function buildKubeadmFlags(input: OidcFormInput): string[] {
  const flags: string[] = [
    `--oidc-issuer-url=${input.issuerUrl}`,
    `--oidc-client-id=${input.clientId}`,
    `--oidc-username-claim=${input.usernameClaim}`,
  ]
  if (input.usernamePrefix) flags.push(`--oidc-username-prefix=${input.usernamePrefix}`)
  flags.push(`--oidc-groups-claim=${input.groupsClaim}`)
  if (input.groupsPrefix) flags.push(`--oidc-groups-prefix=${input.groupsPrefix}`)
  if (input.audience) flags.push(`--oidc-required-claim=aud=${input.audience}`)
  return flags
}

export type ManagedProvider = 'EKS' | 'GKE' | 'AKS'

export interface ProviderInstructionBlock {
  provider: ManagedProvider
  instructions: string[]
}

/**
 * Provider-labeled, high-level guidance for applying an OIDC config on self-managed kubeadm
 * clusters vs. the three big managed offerings. Deliberately hedges on exact CLI/API syntax for
 * the managed offerings, since they change and this is informational text an operator reads, not
 * something Fathom executes.
 */
export function buildProviderInstructions(input: OidcFormInput): ProviderInstructionBlock[] {
  const scopesNote = input.extraScopes
    ? ` Request these scopes from your identity provider when obtaining a token: ${input.extraScopes}.`
    : ''

  return [
    {
      provider: 'EKS',
      instructions: [
        'EKS does not let you pass raw kube-apiserver flags — the control plane is fully managed by AWS.',
        `Instead, configure an EKS OIDC identity provider association (e.g. via "aws eks associate-identity-provider-config" or the console/Terraform equivalent) referencing issuer "${input.issuerUrl}" and client id "${input.clientId}".`,
        'Check the current AWS EKS documentation for the exact command/API shape and any claim-mapping/username-prefix options it exposes, since managed-offering APIs change independently of upstream kube-apiserver flags.' +
          scopesNote,
      ],
    },
    {
      provider: 'GKE',
      instructions: [
        'GKE (Standard and Autopilot) also manages the control plane directly — there is no supported path to set custom --oidc-* kube-apiserver flags yourself.',
        'For GKE, the standard approach is Google-managed authentication (Google identity, or Workload Identity Federation for external OIDC providers) rather than a raw kube-apiserver OIDC flag set.',
        `If you need this exact issuer ("${input.issuerUrl}") recognized, check current GKE docs for its OIDC/identity federation support and constraints — this generated flag set is best treated as a reference for what a self-managed control plane would need, not something you can apply to GKE directly.` +
          scopesNote,
      ],
    },
    {
      provider: 'AKS',
      instructions: [
        'AKS is also a managed control plane and does not accept direct kube-apiserver flag edits.',
        'AKS has its own OIDC/Microsoft Entra ID integration path (Azure AD/Entra integration for AKS, with `kubelogin` as the client-side token helper) rather than free-form --oidc-* flags.',
        `Check current AKS documentation for how to wire up an external issuer like "${input.issuerUrl}" through Entra ID/AKS-native integration, since the exact setup steps and supported providers change over time.` +
          scopesNote,
      ],
    },
    // Self-managed / kubeadm clusters are the case these flags apply to directly — captured here
    // implicitly via the flags themselves; callers show `buildKubeadmFlags()` output alongside
    // these provider blocks with a "self-managed / kubeadm" heading of their own.
  ]
}

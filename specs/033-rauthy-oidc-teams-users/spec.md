# Feature Specification: Rauthy OIDC Identity, Teams & Users

**Feature Branch**: `033-rauthy-oidc-teams-users`
**Created**: 2026-09-24
**Status**: Draft
**Input**: Use Rauthy (OIDC provider, already running on kind context `kind-fathom-oidc-rauthy`, namespace `rauthy`) as the IdP for OAuth/OIDC login to Fathom and to clusters, teams and users (014 Story 3), a role-assignment UI, RBAC-scoped UI (Constitution VI), and LDAP via Rauthy federation instead of a bespoke LDAP client. MVP.md section 14 Tier B, and the Tier C / Tier F items they unblock.

## Context

- 011 (Tier C) is on `main`: Role/Binding views, `RefreshRbacSelfAccess`, `ResolveNamespaceAccess`, an OIDC config *generator* (doc output only, no live login).
- 014 (Tier F) is on `main` for Stories 1-2 (team store). Story 3 (per-user scoped views) is unshipped and gated on an OIDC identity: this feature supplies it.
- Fathom today has no signed-in identity; it uses the kubeconfig's identity. This feature adds an *optional* OIDC identity. Kubeconfig-only use MUST keep working unchanged.

## User Scenarios & Testing

### User Story 1 - Sign in to Fathom with the team IdP (P1)

An operator points Fathom at their organisation's Rauthy issuer and signs in through their system browser. Fathom then knows who they are (username, email, groups) and shows it in the top bar.

**Why P1**: every other story needs an identity. Independently valuable: it delivers a verified "who am I" and sign-out.

**Independent Test**: Against the kind Rauthy, sign in as `alice@fathom.test`; top bar shows `alice`, group `fathom-viewers`; sign out clears it. Needs no cluster.

1. **Given** an issuer URL and client id are configured, **When** the operator chooses Sign in, **Then** Fathom opens the system browser to the Rauthy authorization endpoint using Authorization Code + PKCE (S256), and completes on a loopback callback the backend owns.
2. **Given** a completed login, **Then** the backend validates the ID token (signature via JWKS, `iss`, `aud`, `exp`, `nonce`) and exposes only claims (`sub`, `preferred_username`, `email`, `groups`) to the UI, never the tokens.
3. **Given** the browser cannot be opened or the loopback port is blocked, **Then** Fathom offers the OAuth device-code flow (Rauthy advertises `urn:ietf:params:oauth:grant-type:device_code`).
4. **Given** a login in progress, **When** the operator cancels or the callback times out (120 s), **Then** no partial identity is stored and the error is stated verbatim.
5. **Given** the access token expires (Rauthy dev client: 60 s), **Then** the backend refreshes silently with the refresh token; if refresh fails the identity becomes "signed out" with a visible reason and cluster views fall back to kubeconfig identity, never to a stale token.
6. **Given** sign-out, **Then** local tokens are deleted and the RP-initiated logout (`end_session_endpoint`) is offered.

### User Story 2 - Log in to a cluster as my OIDC identity (P1)

An operator whose cluster API server trusts the Rauthy issuer connects to that cluster as their OIDC user instead of a kubeconfig static credential.

**Independent Test**: On a kind cluster whose API server is configured for the Rauthy issuer, connect as alice; `SelfSubjectAccessReview` reports the prefixed username and her groups.

1. **Given** a signed-in identity and a context marked "authenticate with OIDC", **When** Fathom connects, **Then** the kube client sends the ID token as Bearer, refreshed transparently before expiry.
2. **Given** the API server rejects the token (issuer mismatch, not configured), **Then** the Kubernetes error is shown verbatim with a link to the 011 OIDC generator output for that issuer; Fathom does not retry with any other credential silently.
3. **Given** no OIDC identity is present, **Then** the context connects via kubeconfig exactly as before.
4. **Given** the operator opens the 011 OIDC generator, **Then** issuer, client id, `groups` claim and prefixes are pre-filled from the connected Rauthy discovery document.
5. The ID token is never written to `~/.fathom/` in plaintext; it lives in the OS credential store (Constitution VI).

### User Story 3 - Teams and users from the IdP, per-user scoped views (P2)

An admin sees the users and groups Rauthy manages, treats groups as Fathom teams, and each signed-in teammate sees Fathom scoped to their own resolved access (closes 014 Story 3 / FR-013).

**Independent Test**: alice (`fathom-viewers`) and bob (`fathom-editors`) open the same shared connection; bob sees Scale/Delete enabled, alice sees them disabled with the reason; carol (no group) sees a no-access state.

1. **Given** a signed-in identity, **Then** the Teams view lists the operator's own groups, and for admins lists Rauthy users and groups (read via the Rauthy admin API, backend only).
2. **Given** a team (Rauthy group) is linked to a 014 team-store profile, **Then** only members see/adopt that shared profile in the UI (advisory filter; the store stays the self-hosted store from 014, no new authority).
3. **Given** two teammates with different resolved K8s RBAC on the same cluster, **Then** each sees their own enabled/visible actions (Story 5).
4. **Given** Rauthy is unreachable, **Then** cached claims from the last valid session keep local features working for their remaining validity; team listing shows a non-blocking "IdP unavailable" state.

### User Story 4 - Role assignment UI (P2)

A cluster admin assigns a Rauthy group/user to a Kubernetes role from a picker, instead of typing subject names.

**Independent Test**: Admin picks group `fathom-editors`, ClusterRole `edit`, namespace `fathom-dev`; the diff preview shows a RoleBinding with the prefixed Group subject; confirm applies it; bob's access changes on his next refresh.

1. Subjects are picked from Rauthy users/groups, with the API server's username/groups prefix applied automatically and shown in the preview.
2. The write goes through the existing 011 `ApplyResourceYaml` diff-preview + confirm + conflict path. No new mutation command (Principle I).
3. **Given** the operator lacks RBAC to create bindings, **Then** the control is disabled with the reason (Story 5).
4. **Given** a binding references a group that no longer exists in Rauthy, **Then** it is flagged "orphaned subject" (read-only warning, never auto-deleted).
5. Fathom-level roles (`fathom-viewers`, `fathom-editors`, `fathom-admins`) gate Fathom-only features (publish to team store, manage settings sync). Kubernetes RBAC alone gates cluster actions.

### User Story 5 - RBAC-scoped UI (P2)

The UI hides or disables actions the current identity cannot perform, before the operator tries them (MVP Tier B; Constitution VI).

**Independent Test**: With alice connected, every mutating control (delete, scale, restart, apply, RoleBinding edit, node shell, port-forward) is disabled with a tooltip naming the missing verb/resource/namespace; with bob those enabled by `edit` are active.

1. Backend resolves capabilities per (verb, resource, namespace) using `SelfSubjectRulesReview` per namespace, cached and invalidated on context/identity change and on a 60 s TTL.
2. Denied actions are *disabled with reason* where the resource is visible, *hidden* only when the entire kind is unlistable.
3. The check is advisory: the API server remains the authority. If it denies an action the UI allowed, the verbatim error is shown.
4. Resolution failure fails closed (disabled, "could not verify"), never open.
5. The MCP server (015) reuses the same resolver, so UI and agents agree.

### User Story 6 - LDAP via Rauthy federation (P3)

An org with LDAP/AD wants those users to sign in to Fathom without Fathom containing an LDAP client.

**Independent Test**: In the kind lab, an OpenLDAP user signs in through Rauthy (via the federation bridge) and appears in Fathom with mapped groups; Fathom code contains no LDAP dependency.

1. Fathom only ever speaks OIDC to Rauthy. Federation is IdP-side configuration.
2. Rauthy's upstream providers are OIDC-based; LDAP is bridged by an OIDC-fronting LDAP connector (Dex, see research R4) registered in Rauthy as an upstream provider.
3. LDAP group to `groups` claim mapping is documented and validated by the quickstart.
4. If IdP-side federation is misconfigured, the login simply fails at the IdP; nothing to configure inside Fathom.

### Edge Cases

- Issuer TLS is a private CA: Fathom accepts an operator-supplied CA bundle per issuer, never a global "skip verification".
- Clock skew > 60 s: validation error names skew explicitly.
- Token `groups` claim missing (client lacks `groups` scope): state clearly, treat as zero groups.
- Same email across two issuers: identities are keyed by (issuer, sub), never by email.
- User removed in Rauthy while signed in: next refresh fails, identity becomes signed out.
- Group names with `:` or `/`: escaped in subject prefixing and displayed unescaped.
- Multiple issuers (per cluster): identities are per issuer; one sign-in per issuer.
- Offline: cached, still-valid identity works; expired identity degrades to kubeconfig.

## Requirements

### Functional Requirements

- **FR-001**: Backend MUST implement OIDC Authorization Code + PKCE (S256) with discovery, in Rust, using a maintained OIDC crate; device-code as fallback.
- **FR-002**: Backend MUST validate ID tokens (signature, `iss`, `aud`, `exp`, `nonce`) and cache JWKS with rotation handling.
- **FR-003**: Tokens MUST be stored in the OS credential store; `~/.fathom/` holds only non-secret issuer config. Tokens MUST NOT be logged or sent over the protocol (VI).
- **FR-004**: The protocol MUST expose identity as claims only plus sign-in/out commands, defined in Rust with generated TS types (II).
- **FR-005**: Fathom MUST support an OIDC-authenticated kube client per context, with refresh before expiry, and MUST use kubeconfig only when the operator has not opted the context into OIDC.
- **FR-006**: Fathom MUST NOT silently fall back to a different credential after an OIDC failure on an OIDC-opted context.
- **FR-007**: Fathom MUST read users/groups from Rauthy through the backend using a least-privilege API key (Users:read, Groups:read; group write off by default).
- **FR-008**: Fathom MUST NOT implement an LDAP client, and MUST NOT store LDAP credentials.
- **FR-009**: Role assignment MUST reuse `ApplyResourceYaml` with diff preview, confirmation and resourceVersion conflict detection.
- **FR-010**: Subject naming MUST honour the API server's configured `--oidc-username-prefix` / `--oidc-groups-prefix`, read from operator config and shown in the preview.
- **FR-011**: Backend MUST provide a capability resolver keyed by (identity, context, namespace, verb, resource); all mutating UI controls MUST consume it (disabled + reason), and it MUST fail closed.
- **FR-012**: The MCP server MUST use the same resolver (extends 015 `rbac_guard`).
- **FR-013**: Team = Rauthy group; linking a group to a 014 profile MUST be stored in the team store as non-secret metadata and MUST NOT be treated as an authorization boundary.
- **FR-014**: Existing kubeconfig-only workflows and all existing `~/.fathom/` files MUST load unchanged; new config is additive (VI).
- **FR-015**: Errors from the IdP, the API server and the Rauthy admin API MUST reach the operator verbatim (I).
- **FR-016**: New views MUST follow density scaling (IV) and be keyboard/touch operable at narrow widths.
- **FR-017**: Sign-in, refresh and capability resolution MUST NOT block UI input (>100 ms) nor poll while idle (V): refresh is timer-scheduled off `exp`; capability cache is event-invalidated plus TTL only while the view is open.
- **FR-018**: Dev-lab manifests (Rauthy client/API-key patch, Dex + OpenLDAP) ship as kustomize under `k8s/`; object storage, if ever needed, is RustFS, never MinIO.

### Key Entities

- **OidcIssuerConfig**: issuer URL, client id, optional CA bundle path, scopes, claim names, prefixes. Non-secret, persisted.
- **OidcIdentity**: runtime claims view of a session; tokens only in the credential store.
- **IdpDirectory**: read-only cached snapshot of Rauthy users and groups.
- **RoleAssignment**: derived view joining a Rauthy subject to a K8s (Cluster)RoleBinding.
- **CapabilitySet**: resolver output for one (identity, context, namespace).
- **TeamLink**: Rauthy group name to 014 connection profile id.

## Success Criteria

- **SC-001**: Sign-in against the kind Rauthy completes in under 30 s of operator time, and no token appears in logs, protocol traces or `~/.fathom/`.
- **SC-002**: alice vs bob on one cluster show different enabled action sets (at least one action differs), verified by E2E.
- **SC-003**: 100% of mutating controls are covered by the capability gate (a test enumerates controls and fails on an ungated one).
- **SC-004**: Capability resolution is lazy per namespace; the visible namespace resolves in under 200 ms after the first review.
- **SC-005**: An LDAP user signs in end-to-end in the kind lab with zero Fathom LDAP code.
- **SC-006**: Kubeconfig-only installs upgrade with no config change and no behavioural difference.

## Assumptions

- Rauthy 0.36.2 as deployed: issuer `https://auth.localtest.me/auth/v1/`, `groups` scope and claim, S256 PKCE, device-code advertised, dev clients/users/groups bootstrapped from ConfigMap `rauthy-bootstrap` (alice, bob, carol; groups fathom-viewers, fathom-editors).
- The existing `fathom` Rauthy client is confidential with an https web redirect; a new public native client (`fathom-desktop`) is required (secrets cannot ship in a desktop binary).
- Cluster admins configure API servers to trust the issuer (011 generator instructs; Fathom does not mutate API server flags).
- Mobile and browser builds stay out of scope. Hosted SaaS stays a non-goal.
- No object storage is required; if research later adds one it is RustFS.

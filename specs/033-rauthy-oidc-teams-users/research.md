# Research: 033-rauthy-oidc-teams-users

## Live Rauthy inspection (context `kind-fathom-oidc-rauthy`, ns `rauthy`)

- Deployment `rauthy`, 1 replica, image `ghcr.io/sebadob/rauthy:0.36.2`, strategy Recreate, data on `emptyDir` (state is lost on pod recreation; the bootstrap ConfigMap reseeds), embedded Hiqlite (`HQL_*`), `PROXY_MODE=true`, `PUB_URL=auth.localtest.me`, secrets from `rauthy-secrets`. Service `rauthy:8080`, Ingress class `nginx` host `auth.localtest.me` (port 80 redirects 308 to 443).
- Discovery (`/auth/v1/.well-known/openid-configuration`): issuer `https://auth.localtest.me/auth/v1/`; scopes `openid profile email groups address phone`; claims include `preferred_username email given_name family_name roles groups custom`; grants `authorization_code client_credentials password refresh_token device_code token-exchange`; PKCE `plain, S256`; `device_authorization_endpoint`, `end_session_endpoint`, `jwks_uri` present.
- ConfigMap `rauthy-bootstrap`: `clients.json` (client `fathom`, confidential, redirect `https://fathom.localtest.me/auth/callback`, RS256, `access_token_lifetime` 60 s, scopes incl. `groups`, S256 only), `groups.json` (`fathom-viewers`, `fathom-editors`), `users.json` (alice in viewers, bob in editors, carol no group; role `user`), `api_keys.json` (key `e2e` with Users/Sessions/Clients/Groups CRUD).
- Implications:
  - The `groups` claim already works; K8s `--oidc-groups-claim=groups` needs nothing extra.
  - The `fathom` client is unusable for a desktop app (confidential + https redirect). Add `fathom-desktop`: public, PKCE S256, loopback redirect, refresh_token + device_code.
  - The 60 s access-token lifetime forces proactive refresh (R6): good test pressure.
  - Plaintext dev secrets and a full-CRUD `e2e` API key live in the ConfigMap: dev lab only; the plan ships a read-only key (R5) and labels the lab non-production.
  - `emptyDir` data: tests must not rely on UI-created users; change bootstrap files instead.

## Decisions

### R1. Login flow for a desktop app
Decision: Authorization Code + PKCE via the system browser and a backend loopback listener (127.0.0.1, ephemeral port); device-code fallback. Rationale: RFC 8252; no embedded webview (differs per Tier 1 platform, phishing surface). Rejected: password grant (deprecated; breaks MFA/passkeys).
Verify in lab: whether Rauthy accepts `http://127.0.0.1:<random port>` redirects (RFC 8252 s7.3). If exact match only, register a small fixed port range and fall back to device-code. Tracked as risk.

### R2. Crates
`openidconnect` (discovery, PKCE, ID-token validation) and `keyring` (Secret Service / Credential Manager / Keychain: matches Tier 1). Rejected: hand-rolled JWT validation with `jsonwebtoken` (MVP section 14 design constraint prefers existing crates). If no keyring is available (headless Linux), tokens stay in memory only and the UI says so; no plaintext file fallback.

### R3. Cluster login
Fathom is the token source: the kube client for an OIDC-opted context attaches a Bearer layer reading the current ID token (R6). API server side is standard: `--oidc-issuer-url/--oidc-client-id/--oidc-username-claim/--oidc-groups-claim/--oidc-*-prefix`, or structured `AuthenticationConfiguration` where the cluster version supports it (CEL claim mapping, multiple issuers), which is preferred. The ID token `aud` must match the API server's client id: use `fathom-desktop` as that id, or a dedicated `kube-apiserver` client that `fathom-desktop` may request (verify in lab).
Kind gotcha: `auth.localtest.me` resolves to 127.0.0.1 inside the node, not the ingress. Lab fix: node `/etc/hosts` entry to the ingress-controller/Rauthy service IP and `--oidc-ca-file` with the ingress CA, mounted through kubeadm `extraVolumes`. Documented in the quickstart.

### R4. LDAP via Rauthy
Finding: Rauthy has no native LDAP client; its upstream auth providers are OIDC-family. Design: an LDAP-to-OIDC bridge (Dex `ldap` connector) registered in Rauthy as an upstream OIDC provider. Fathom sees only Rauthy. Rejected: Keycloak bridge (heavier), a bespoke LDAP client in Fathom (out by requirement).
Verify in lab: Rauthy's upstream-provider mapping of Dex `groups` to Rauthy groups. If unsupported, a small IdP-side reconciler job (Rauthy admin API, group write key) is a follow-up, not Fathom code. Lab: OpenLDAP + Dex in `k8s/rauthy-ldap-lab/`.

### R5. Reading users and groups
Rauthy admin REST API with an API key limited to `Users:read`, `Groups:read`; group write is off by default. Called only from the backend; key stored in the credential store, entered by the admin. Non-admins get only their own claims. Rejected: SCIM (not verified in 0.36.2), claims-only (cannot feed the admin picker).

### R6. Token refresh
One tokio task per issuer sleeps until `exp - 20 s`, refreshes, swaps the token in an `ArcSwap`. No polling. Rotating refresh tokens stored atomically. Failure emits `OidcSessionEnded { reason }`.

### R7. Capability resolver
`SelfSubjectRulesReview` per namespace (one call returns all rules), plus `SelfSubjectAccessReview` for cluster-scoped checks and when the rules review is `incomplete`. Extends `src/backend/rbac_access.rs`. Cache key (context, identity sub, namespace), TTL 60 s, invalidated on identity/context change and after a successful RBAC apply. Lazy per visible namespace (Principle V). Fails closed.

### R8. Team authority
Teams are Rauthy groups. The 014 store is a self-hosted file store and cannot enforce access, so group-to-profile links are advisory visibility filters. Authorization is Kubernetes RBAC.

### R9. Fathom-level roles
Groups `fathom-viewers|editors|admins` (configurable) gate Fathom-only features (team publish, settings sync). Never used to grant cluster access.

### R10. Helper manifests
`./k8s` does not exist on `main` (only `dev/sandbox/`). The plan creates `k8s/` with kustomize bases: `rauthy-clients` (adds `fathom-desktop` and a read-only API key), `rauthy-ldap-lab` (OpenLDAP + Dex), `kind-oidc-apiserver` (kubeadm patch and notes). Apply with `kubectl --context kind-fathom-oidc-rauthy apply -k k8s/<base>`. RustFS is the only object store considered (not needed).

## Open risks
1. Loopback redirect matching in Rauthy (R1).
2. LDAP group mapping Dex to Rauthy (R4).
3. Kind API server reaching the issuer (R3).
4. Keyring absent on some Linux setups (R2).
5. `emptyDir` Rauthy state in the lab.

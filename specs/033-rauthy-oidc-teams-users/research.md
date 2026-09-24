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

## T004 lab spike results (kind-fathom-oidc-rauthy, Rauthy 0.36.2, k8s v1.32.2, run 2026-09-24)

Method: registered `fathom-desktop` (k8s/rauthy-clients), drove a real login as alice/bob/carol with headless Chromium against a 127.0.0.1 ephemeral-port listener (PKCE S256), exchanged the code, decoded tokens, refreshed, then presented the ID token to the kind API server.

### R1 loopback redirect matching: VERIFIED, with constraints
- Rauthy matches redirect URIs by exact string, except that a **trailing `*` is a prefix wildcard** (only at the end; `http://127.0.0.1:*/callback` is rejected as "Invalid redirect uri"). Registered `http://127.0.0.1:*` and `http://localhost:*`: any port and path on those hosts is accepted (tested ports 53211, 60000, arbitrary paths). `http://127.0.0.1` without a port requires `http://127.0.0.1*`, so the client uses the explicit-port form only.
- `http://[::1]:port` is **rejected by Rauthy input validation** (redirect_uri regex forbids brackets): the listener MUST bind IPv4 127.0.0.1 only. `localhost` works as a host but Fathom uses the literal 127.0.0.1 (RFC 8252 s8.3).
- Residual risk: the prefix match cannot pin the path, and `http://127.0.0.1:` + attacker-controlled suffix is a prefix match. Mitigated because the client is public with PKCE S256 (a stolen code is useless without the verifier, which never leaves the process); Fathom also checks `state` and only ever builds the redirect itself. Recorded in the plan risks.
- No fixed-port fallback needed. Device code remains the fallback when the loopback bind fails.
- Full flow result: callback with `code` + `state`, token exchange without a client secret succeeds.

### Tokens and R6 refresh
- ID token: RS256 (client set to RS256; the default for new Rauthy clients is **EdDSA, which kube-apiserver does not support**, so the bootstrap pins RS256). `iss` = discovery issuer (with trailing slash), `aud` = `fathom-desktop`, `azp` = `fathom-desktop`, `nonce` echoed, `sub` is an opaque id, `preferred_username`, `email`, `roles`, `groups` present. Lifetime 60 s (`exp - iat`).
- Access token is also a JWT with `aud` = `fathom-desktop`; not used for the API server (ID token is, R3).
- Refresh: returns a new ID token (same aud), new access token and a **rotated refresh token**. Re-presenting the previous refresh token immediately after rotation still returned 200 (short reuse grace window in Rauthy), so Fathom must always persist the newest token but cannot rely on Rauthy to reject a stale one.
- carol (no group): ID token `groups: []`, but the **access token has `groups: [""]`** (an empty-string group). Fathom must drop empty group names when reading any token/claim and never render or map `""`.
- Device flow: `POST /oidc/device` for the public client returns `user_code`, `verification_uri`, `verification_uri_complete`, `expires_in` 300, `interval` 5. Works without a client secret.
- Discovery `end_session_endpoint` = `.../oidc/logout`; `id_token_signing_alg_values_supported` = RS256/RS384/RS512/EdDSA.

### R3 API server aud handling: VERIFIED
- With the legacy flags (`--oidc-client-id=fathom`) the API server rejects a `fathom-desktop` ID token: `oidc: verify token: oidc: expected audience "fathom" got ["fathom-desktop"]` (surfaced to kubectl as Unauthorized). So `aud` must match exactly.
- Fix verified: structured `AuthenticationConfiguration` (`apiserver.config.k8s.io/v1beta1`, enabled by default on 1.32) with `audiences: [fathom-desktop, fathom]`, `audienceMatchPolicy: MatchAny`, per-issuer `certificateAuthority`, claim mappings username `preferred_username` prefix `oidc:`, groups `groups` prefix `oidc:`. `kubectl auth whoami` with alice's ID token: `oidc:alice`, groups `[oidc:fathom-viewers system:authenticated]`; carol: `oidc:carol`, `[system:authenticated]` (no `""` group leaks because the ID token is used). alice can list pods (existing RBAC); carol is Forbidden. `kubectl auth can-i --list` (SelfSubjectRulesReview) works with the OIDC identity, which R7 relies on.
- `--oidc-*` flags and `--authentication-config` are mutually exclusive: `k8s/kind-oidc-apiserver/apply.sh` removes the flags and adds the file. The file lives under `/etc/kubernetes/pki/` on the node because `/etc/fathom-oidc` is a read-only bind mount.
- Decision: use `fathom-desktop` as the audience; no dedicated `kube-apiserver` client is required.
- Gap found: the operator's 011 generator still emits the legacy `--oidc-*` flag form. Task T022 prefill must offer the AuthenticationConfiguration form (needed to accept two audiences); keep flags for clusters < 1.30.

### R4 upstream group mapping: NOT SUPPORTED (schema evidence; live confirmation deferred to T045)
- Rauthy 0.36.2 `auth_providers` (upstream OIDC) columns carry only `admin_claim_path/admin_claim_value` and `mfa_claim_path/mfa_claim_value`; there is no group-mapping field, and no LDAP client. So Dex `groups` cannot be mapped into Rauthy groups by configuration.
- Consequence for Phase 8: T045 takes the documented fallback: an IdP-side reconciler (a CronJob in `k8s/rauthy-ldap-lab/` calling the Rauthy admin API with a group-write key to mirror LDAP group membership from Dex/LDAP into Rauthy groups). No Fathom code. The binary also references `application/scim+json`; a SCIM push into Rauthy may be a better reconciler target and should be probed in T045 before writing the job.
- Probe limitation: the lab `e2e` API key lacks `AuthProviders` permission, so the provider API could not be exercised live.

### Lab bootstrap findings (affect T001 and every E2E)
- Rauthy bootstrap files are read only on a fresh data dir and are all-or-nothing: an invalid file panics the server at start (crash loop). Two mistakes hit here: grant names must be URNs (`urn:ietf:params:oauth:grant-type:device_code`, not `device_code`), and API key secrets must be >= 64 characters. Recreate the pod (`delete pod`, not `rollout restart`, which keeps the emptyDir) after changing the ConfigMap.
- After the fix, `users.json` (alice/bob/carol with the repo's known dev password) and `clients.json` seed correctly, including `fathom-admins` and the read-only `fathom-read` key (HTTP 200 on `GET /users`, `Users:read` + `Groups:read` only).
- Applying the kustomization over the existing ConfigMap works (kubectl merges keys); `users.json` is preserved.

## T044-T046 LDAP lab results (kind-fathom-oidc-rauthy, run 2026-09-24)

- `kubectl apply -k k8s/rauthy-ldap-lab` deploys OpenLDAP 1.5.0, Dex v2.41.1 (LDAP connector, static client `rauthy`), a `ldap-seed` Job and the `group-reconciler` CronJob (ns `ldap-lab`). VERIFIED live: OpenLDAP seeded (users dave/erin/frank; groups fathom-editors {dave}, fathom-viewers {erin, dave}); Dex serves discovery at `http://dex.ldap-lab.svc.cluster.local:5556/dex`.
- The osixia image's custom-ldif hook exits with status 68 on this seed (crash loop), so seeding is a `ldapadd -c` Job instead.
- `scripts/check-no-ldap.sh` (wired into CI `check`) passes: `cargo tree` has no LDAP crate.
- NOT VERIFIED live: Rauthy upstream-provider registration and the reconciler run. They need the `ldap-lab` API key (AuthProviders/Users/Groups, added to `k8s/rauthy-clients/api_keys.json`), which is bootstrap-only and needs a Rauthy pod recreate; that step was blocked in this session. `register-provider.sh` and `reconciler.sh` are written against the Rauthy 0.36.2 REST shape (`/auth/v1/providers`, `/auth/v1/groups`, `/auth/v1/users/{id}`) and their request bodies are unprobed; expect field adjustments on first run.
- Decision stands: reconciler is the group path (no upstream group mapping in Rauthy).

## Open risks
1. ~~Loopback redirect matching in Rauthy (R1)~~ verified in T004 (IPv4 only, trailing-wildcard registration).
2. LDAP group mapping Dex to Rauthy (R4): confirmed unsupported; reconciler needed (T045).
3. ~~Kind API server reaching the issuer (R3)~~ resolved; aud must match, use AuthenticationConfiguration.
4. Keyring absent on some Linux setups (R2).
5. `emptyDir` Rauthy state in the lab.

# Hosted Server Mode: Operator Guide

Hosted server mode (spec `031-hosted-server-mode`) runs Fathom as a single web service inside a
cluster. People sign in through your OIDC provider and use the same UI as the desktop app in a
browser. Every request to the Kubernetes API is made **as the signed-in person**, so cluster RBAC
decides what each of them can see and do. Fathom itself grants nothing.

The binary is `fathom-server` (crate `server/`); the chart is `deploy/helm/fathom`. The desktop
app and devbridge are unaffected: none of the settings below apply to them.

- [Architecture in one paragraph](#architecture-in-one-paragraph)
- [Install](#install)
- [OIDC provider setup](#oidc-provider-setup)
- [Kubernetes API server setup (pass-through)](#kubernetes-api-server-setup-pass-through)
- [Impersonation model (managed clusters)](#impersonation-model-managed-clusters)
- [Ingress, TLS, WebSocket and Origin](#ingress-tls-websocket-and-origin)
- [Audit log](#audit-log)
- [Sessions, sign-out and revocation](#sessions-sign-out-and-revocation)
- [Configuration reference](#configuration-reference)
- [Troubleshooting](#troubleshooting)

## Architecture in one paragraph

The browser holds only an opaque `__Host-fathom_session` cookie (HttpOnly, Secure,
SameSite=Lax). Tokens live in server memory, never in the browser, on disk or in logs. Each
signed-in person gets their own backend instance (the same engine the desktop app runs) with a
private state directory `<state>/users/<sha256(issuer|sub)[..16]>/`. Commands that would touch
shared state (kubeconfig editing, local integrations, desktop-only features) are refused in
hosted mode. Fathom renews tokens silently in the background, and a failed renewal ends that
person's live views within the revalidation interval (default 60 s, well under the 5 min
revocation target). If the OIDC provider is unreachable at startup, the server refuses to become
ready: there is never an unauthenticated listener.

## Install

```bash
helm install fathom deploy/helm/fathom \
  --namespace fathom --create-namespace \
  --set externalUrl=https://fathom.example.com \
  --set oidc.issuer=https://idp.example.com/realms/ops \
  --set oidc.clientId=fathom \
  --set oidc.existingSecret=fathom-oidc \
  --set ingress.enabled=true --set ingress.className=nginx \
  --set persistence.enabled=true
```

- `externalUrl` must be `https://`. It fixes the redirect URI
  (`<externalUrl>/auth/callback`), the only accepted `Origin`, and the cookie scope.
- `oidc.existingSecret` names a Secret with key `client-secret`. Omit it for a public client
  (PKCE is always used).
- `oidc.caSecret` names a Secret with key `ca.crt` when the issuer uses a private CA.
- `replicaCount` is fixed at 1: sessions are held in memory. A restart signs everyone out.
- Enable `persistence` if you want the [audit log](#audit-log) and per-user preferences to
  survive pod restarts. Without it they live in an `emptyDir`.

A local end-to-end walk-through (kind, Rauthy or Keycloak, sample users) is in
`specs/031-hosted-server-mode/quickstart.md`.

## OIDC provider setup

Register one **confidential or public web client** with:

| Setting | Value |
|---|---|
| Grant | Authorization code + PKCE (S256) + refresh token |
| Redirect URI | `https://fathom.example.com/auth/callback` |
| Post-logout redirect URI | `https://fathom.example.com/` |
| Scopes | `openid profile email groups` (override with `oidc.scopes`; must include `openid`) |
| Token lifetime | Access/ID tokens 5-15 min; refresh tokens enabled |

Fathom reads two claims from the ID token: a **username** claim (`claims.username`, default
`preferred_username`) and a **groups** claim (`claims.groups`, default `groups`). The groups
claim must be a JSON array of strings. The same claims must be what your API server (or, in the
impersonation model, what Fathom) maps to Kubernetes users and groups.

### Claim tables

| Provider | Issuer URL | Username claim | Groups claim | Group mapper |
|---|---|---|---|---|
| Rauthy | `https://<host>/auth/v1` | `preferred_username` | `groups` | Built in: add the `groups` scope to the client and assign groups to users. |
| Keycloak | `https://<host>/realms/<realm>` | `preferred_username` | `groups` | Client scope `groups` with a *Group Membership* mapper, "Full group path" **off**, "Add to ID token" **on**. |
| Dex | `https://<host>/dex` | `preferred_username` or `email` | `groups` | Groups come from the upstream connector (LDAP, GitHub teams...). Request the `groups` scope; add `offline_access` to `oidc.scopes` for refresh tokens. |
| Microsoft Entra ID | `https://login.microsoftonline.com/<tenant>/v2.0` | `preferred_username` (UPN) or `oid` | `groups` | App registration, Token configuration, *Add groups claim* (security groups). Values are **object IDs**, not names; bind RBAC to IDs. Users in >200 groups get an overage claim instead: use app roles (`claims.groups=roles`) for large tenants. |
| Okta | `https://<org>.okta.com/oauth2/<server-id>` | `preferred_username` or `email` | `groups` | Authorization server, Claims, add `groups` (ID token, *Always*, filter e.g. regex `fathom-.*`). The org authorization server does not issue groups: use a custom one. |

`claims.usernamePrefix` / `claims.groupsPrefix` (chart default `oidc:`) are prepended before the
names reach Kubernetes, so a provider group `fathom-editors` is `oidc:fathom-editors` in RBAC.
They must match the API server's `--oidc-username-prefix` / `--oidc-groups-prefix` in the
pass-through model.

## Kubernetes API server setup (pass-through)

`accessModel: passthrough` (default) sends the person's own token to the API server as a bearer
token. The API server must trust the same issuer:

```text
--oidc-issuer-url=https://idp.example.com/realms/ops
--oidc-client-id=fathom
--oidc-username-claim=preferred_username
--oidc-username-prefix=oidc:
--oidc-groups-claim=groups
--oidc-groups-prefix=oidc:
```

(or the equivalent `AuthenticationConfiguration` / structured authentication file). With
`kubeToken: id` (default) the ID token is sent; `kubeToken: access` sends the access token for
providers whose access tokens are JWTs with a matching audience.

Then bind RBAC to the prefixed names, e.g.
`kubectl create rolebinding editors --clusterrole=edit --group=oidc:fathom-editors -n demo`.

## Impersonation model (managed clusters)

Managed control planes (EKS, GKE, AKS, most hosted distributions) often do not let you add an
OIDC issuer to the API server. Use `accessModel: impersonation`:

- Fathom authenticates to the API server with its own ServiceAccount and sends
  `Impersonate-User` / `Impersonate-Group` headers carrying the signed-in person's prefixed
  username and groups. RBAC is evaluated for the impersonated identity, exactly as in
  pass-through.
- The chart creates a ClusterRole allowing only `impersonate` on `users` and `groups` and binds
  it to Fathom's ServiceAccount. The ServiceAccount has **no** read or write access of its own.
  Narrow it further with `impersonation.resourceNames`.
- Treat that ServiceAccount as highly privileged: anyone who can exec into the pod or read its
  token can act as any user. Keep the namespace locked down, enable `networkPolicy`, and do not
  grant `impersonate` on `serviceaccounts` or `uids`.
- A person whose ID token carries no groups is refused with an explicit "no groups" state
  rather than being silently impersonated with fewer permissions. Fix the provider's group
  mapper.
- The audit log always records the **real** signed-in person, never the ServiceAccount.

## Ingress, TLS, WebSocket and Origin

- Terminate TLS at the ingress (or in front of it). Fathom sets HSTS and `__Host-` cookies and
  will not work over plain HTTP.
- The UI uses a WebSocket at `/ws`. The ingress must allow `Upgrade` and keep idle connections
  open for at least a few minutes. For ingress-nginx:
  ```yaml
  ingress:
    annotations:
      nginx.ingress.kubernetes.io/proxy-read-timeout: "3600"
      nginx.ingress.kubernetes.io/proxy-send-timeout: "3600"
  ```
  Traefik and most Gateway API implementations upgrade WebSockets without extra settings.
- `/ws` and `/auth/logout` accept only requests whose `Origin` header exactly equals the
  scheme, host and port of `externalUrl`. If the ingress or a CDN serves Fathom under another
  hostname, or rewrites `Origin`, connections are refused. Serve Fathom under exactly one host.
- Do not path-prefix Fathom (`/fathom/...`): it must own the host root, because `__Host-`
  cookies require `Path=/`.
- `/healthz` (liveness) and `/readyz` (readiness) are unauthenticated. Do not expose them
  outside the cluster if you consider the readiness reason text sensitive.

## Audit log

Every mutating action (scale, restart, delete, cordon, apply...) is recorded, including
refused ones, following the spec 026 outcome rules: a refusal is recorded as a failure with the
API server's reason.

| Aspect | Behavior |
|---|---|
| Location | `<state>/audit/audit-log.jsonl` (`/var/lib/fathom/audit/...` in the chart). One JSON object per line; directory `0700`, file `0600`. |
| Attribution | Each entry has an `actor` (`sub`, `username`, `groups`) taken from the verified ID token, in both access models. |
| Integrity | Append-only. There is no edit or delete operation in the UI or the protocol. A torn or corrupt line is skipped when read; it never breaks the log. |
| Retention | `audit.retention` (`FATHOM_AUDIT_RETENTION`, default 10 000). When the file reaches twice the limit, it is compacted to the newest `retention` entries. |
| Who sees what | People see only their own entries (matched by `sub`, not username). Members of `audit.viewerGroups` (`FATHOM_AUDIT_VIEWER_GROUPS`, raw provider group names without prefix) see everyone's. |
| Log mirror | Each entry is also emitted as a structured `tracing` event (target `fathom::audit`) on stdout, so your log pipeline can keep it longer and tamper-evident. Tokens never appear in either form. |

For compliance-grade retention, ship the `fathom::audit` log lines to your log store; the file
is a convenience view, not a system of record. Without `persistence.enabled` the file is lost on
restart; the log mirror is not.

## Sessions, sign-out and revocation

- Sessions end after `session.absolute` (default 28800 s, 8 h) or after `session.idle`
  (default 1800 s, 30 min) without activity. Both are whole seconds.
- Tokens are refreshed in the background at least every `session.revalidateSeconds`. A refused
  refresh (user disabled, session revoked at the provider) ends the person's live views and
  shows a sign-in prompt. `session.refreshGraceSeconds` tolerates a briefly unreachable
  provider.
- Sign-out ends the server session and, when the provider advertises `end_session_endpoint`,
  sends the browser to it with `client_id` and `post_logout_redirect_uri`.
  **`id_token_hint` is deliberately omitted in pass-through + `kubeToken: id`**: there the ID
  token is a live cluster credential, and it would otherwise end up in browser history and
  provider access logs. Most providers (Keycloak, Rauthy, Okta, Entra) accept `client_id`
  instead; some show a confirmation page rather than redirecting back immediately.
  With `kubeToken: access` or impersonation the hint is sent.

## Configuration reference

The chart maps values to environment variables; `fathom-server` can also be run directly with
them.

| Env var | Chart value | Default | Notes |
|---|---|---|---|
| `FATHOM_EXTERNAL_URL` | `externalUrl` | required | `https://` only |
| `FATHOM_OIDC_ISSUER` | `oidc.issuer` | required | discovery must succeed at startup |
| `FATHOM_OIDC_CLIENT_ID` | `oidc.clientId` | required | |
| `FATHOM_OIDC_CLIENT_SECRET[_FILE]` | `oidc.clientSecret` / `existingSecret` | none | public client if unset |
| `FATHOM_OIDC_CA_FILE` | `oidc.caSecret` | system roots | PEM bundle |
| `FATHOM_OIDC_SCOPES` | `oidc.scopes` | `openid,profile,email,groups` | |
| `FATHOM_CLAIM_USERNAME` / `FATHOM_CLAIM_GROUPS` | `claims.*` | `preferred_username` / `groups` | |
| `FATHOM_USERNAME_PREFIX` / `FATHOM_GROUPS_PREFIX` | `claims.*Prefix` | empty (chart: `oidc:`) | |
| `FATHOM_ACCESS_MODEL` | `accessModel` | `passthrough` | or `impersonation` |
| `FATHOM_KUBE_TOKEN` | `kubeToken` | `id` | or `access` |
| `FATHOM_AUDIT_VIEWER_GROUPS` | `audit.viewerGroups` | none | comma-separated |
| `FATHOM_AUDIT_RETENTION` | `audit.retention` | `10000` | must be > 0 |
| `FATHOM_LIMIT_*` | `limits.*` | see `values.yaml` | per-user and global caps |
| `FATHOM_SESSION_ABSOLUTE` / `_IDLE` | `session.*` | `28800` / `1800` | whole seconds |
| `FATHOM_REFRESH_GRACE_SECONDS` | `session.refreshGraceSeconds` | `120` | |
| `FATHOM_REVALIDATE_SECONDS` | `session.revalidateSeconds` | `60` | |
| `FATHOM_STATE_DIR` | fixed | `/var/lib/fathom` | per-user state and audit |
| `FATHOM_LISTEN` | fixed | `0.0.0.0:8080` | |

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Pod never ready, log says `OIDC discovery failed` | Wrong issuer URL, or a private CA without `oidc.caSecret`. |
| Sign-in loops back to the provider | Redirect URI mismatch, or the ingress host differs from `externalUrl`. |
| UI loads but stays "connecting" | Ingress does not upgrade `/ws`, or `Origin` differs from `externalUrl`. |
| Everything shows "No access" / 401 | API server does not trust the issuer, or the username/groups prefixes differ between Fathom, the API server and your RoleBindings. |
| Groups ignored (Entra) | Groups arrive as object IDs; bind RBAC to the IDs, or use app roles. |
| Impersonation: "missing groups" state | The groups claim is absent from the ID token; fix the provider mapper. |

# Contract: Hosted HTTP and WebSocket surface

Base = configured `external_url`. Cookie `__Host-fathom_session` (HttpOnly, Secure, SameSite=Lax,
Path=/). All `/api/*` responses `Cache-Control: no-store`.

| Method + path | Auth | Purpose |
|---|---|---|
| `GET /` and static assets | session for `/`; assets public | Serves UI bundle. Unauthenticated `GET /` -> 302 `/auth/login?return=<path>`. |
| `GET /auth/login` | none | Creates state/nonce/PKCE, sets `__Host-fathom_login`, 302 to IdP authorization endpoint (`code` flow, `S256`). `return` must be a same-origin path (open-redirect check). |
| `GET /auth/callback` | login cookie | Validates `state`, exchanges code with verifier, validates ID token (sig, iss, aud, exp, nonce), builds Identity, creates Session, sets session cookie, clears login cookie, 302 to return path. Errors render an actionable page (no token echo). |
| `POST /auth/logout` | session + CSRF + Origin | Ends session, aborts user backend if last session, clears cookie, returns `{ "endSessionUrl": string \| null }` for RP-initiated logout. |
| `GET /api/session` | session | `{ username, groups, csrf, expiresAt, idleTimeoutSec, accessModel, mode: "hosted", limits, refreshCount? }`. 401 JSON `{ "error": "unauthenticated", "loginUrl": "/auth/login" }` when none. Never returns tokens. |
| `GET /healthz` | none | Liveness: process up. |
| `GET /readyz` | none | Readiness: OIDC discovery loaded, JWKS cached, home cluster reachable; 503 with reason otherwise (FR-019). |
| `GET /ws` | session + Origin | WebSocket upgrade; see below. |

Rejections: missing/invalid session on `/ws` -> 401 before upgrade; bad Origin -> 403; CSRF
failure -> 403; limit -> 429 with `{ "error": "limit", "kind": ... }`.

## WebSocket frames

Text JSON frames identical to `DevBridgeClientMsg` / `DevBridgeServerMsg`
(`src/protocol/v1/bridge_types.rs`): `send`, `subscribe`, `sent`, `subscribed`, `batch`, `error`.
No `?token=`. `subscribed.info` carries `mode`, `identity`, `limits`. Close codes: 4401 session
ended (client goes to sign-in), 4429 tab limit, 4403 origin, 1001 server shutdown.
`Forbidden(reason)` replies are used for filtered commands and per-user limits; the connection
stays up.

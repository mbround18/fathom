# Security Review: Hosted Server Mode (T055)

**Date**: 2026-09-26. **Scope**: `server/` (`fathom-server`), the hosted seams in `src/backend`
(`access`, `persistence`, `audit_log`) and `deploy/helm/fathom`. **Method**: manual code
review against the checklist below, a grep sweep, new regression tests, `cargo audit`.
The interactive `/security-review` command could not be run from the agent that did this
review; run it on the merge PR as a second pass.

## Findings

| # | Severity | Finding | Status |
|---|---|---|---|
| S1 | **Critical** | Per-user state root was process-global and first-wins (`OnceLock`). Every hosted user's backend resolved `config.json`, `integrations/credentials.json`, agent conversations and the audit log under the **first** signed-in user's directory: cross-user data exposure (FR-005/FR-013). | **Fixed.** Root is now per thread (`persistence::set_thread_state_root`) and applied to every worker and blocking thread of each user's runtime (`scope_runtime_to_state_root`). `fathom-server` calls `require_scoped_state_root()`, so an unscoped path is an error instead of falling back to `~/.fathom`. Test: `persistence::state_root_is_scoped_to_its_threads`. |
| S2 | High | Hosted used the desktop `LocalFileSink`: one shared log, no actor filtering, every user could read every other user's audit entries. | **Fixed** by `server/src/audit.rs` (`HostedAuditSink`, T048-T050). Visibility by `actor.sub` (not username, so a same-name account at the IdP cannot read someone else's entries), all entries only for `audit_viewer_groups`. Tests: `audit::tests::visibility_by_actor_and_viewer_groups`, `server/tests/audit_visibility.rs`. |
| S3 | High | Sign-out URL returned to the browser carried `id_token_hint=<ID token>`. In pass-through + `kubeToken: id` that token is a live cluster credential (valid at the API server until expiry) and would land in browser history, page scripts and IdP access logs (FR-011, FR-017). | **Fixed.** The hint is omitted in that mode; `client_id` identifies the RP instead (RP-Initiated Logout 1.0 §2). Test: `http_ws::logout_requires_csrf_and_origin_then_ends_the_session`. **Follow-up for the live suites (T038/T045):** confirm Rauthy and Keycloak still redirect back without the hint; some IdPs show a confirmation page instead. |
| S4 | Medium | `Session` derived `Debug`, printing the session id (the cookie value) and CSRF token; `LoginState` derived `Debug`, printing `state`, `nonce` and the PKCE verifier. Any `{:?}` log line would leak a usable session. | **Fixed** with redacting `Debug` impls. Test: `http::tests` redaction test now asserts the id, CSRF and login secrets are absent. |
| S5 | Medium | `safe_return_path` accepted non-ASCII and some non-visible characters; the value goes into a `Location` header via `Redirect::to`, which panics on bytes `HeaderValue` rejects (handler panic / header issues). | **Fixed.** Only visible ASCII (`is_ascii_graphic`) is accepted, otherwise `/`. Tests: `csrf::tests` (`/café`, `/a b`, `/x\x7f`). |
| S6 | Low | `/readyz` is unauthenticated and returns the readiness reason text (e.g. "no in-cluster config: ..."). | Accepted. Reveals configuration state only, no secrets; needed by probes. Documented in `docs/HOSTED.md`. |
| S7 | Low | CSP allows `style-src 'unsafe-inline'` (UI component library injects styles). | Accepted. `script-src` stays `'self'`; no inline script. Revisit if the UI drops runtime style injection. |
| S8 | Low | Requests without `Sec-Fetch-Site` are accepted. | Accepted. The exact `Origin` match and CSRF header still apply to every state-changing endpoint and `/ws`. |
| S9 | Info | `cargo audit` advisories (see below). None reachable in the hosted server's use. | Tracked, not fixed here. |

## Checklist

### Token handling and redaction
- [x] All tokens (`id_token`, `access_token`, `refresh_token`, client secret) are `SecretString`; `Debug` prints `[REDACTED]`.
- [x] `TokenCell` stores the bearer as a `HeaderValue` with `set_sensitive(true)`; `Credentials`, `AuthLayer`, `AuthService` and `Impersonation` `Debug` output carries no token.
- [x] `Session` and `LoginState` `Debug` redacted (S4).
- [x] `expose_secret()` is called only to build the bearer header, the refresh grant, the client secret for the token endpoint and (outside S3's mode) the logout hint.
- [x] No `tracing` call formats a token, cookie, CSRF value or verifier. `refresh.rs` logs only the `AuthError` text ("token contains characters...").
- [x] `/api/session` returns identity and limits only, no tokens (`http_ws::session_endpoint_is_token_free_and_headers_are_set`).
- [x] Audit entries and their `fathom::audit` tracing mirror carry actor (`sub`, `username`, `groups`), action and target only; `audit_visibility` asserts no token appears in the file.
- [x] OIDC callback errors do not echo provider text or parameters (`http_ws::callback_with_bad_state_is_rejected_without_echo`).

### Cookies and headers
- [x] `__Host-fathom_session`: `Secure`, `HttpOnly`, `SameSite=Lax`, `Path=/`, no `Domain` (`http_ws::login_sets_hardened_cookies_and_serves_the_app`).
- [x] CSP: `default-src 'self'`, `script-src 'self'`, `connect-src 'self'` + `wss://<host>`, `frame-ancestors 'none'`, `base-uri 'self'`, `form-action 'self'`, `object-src 'none'`; S7 noted.
- [x] `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`, `Permissions-Policy`.
- [x] HSTS (`max-age=31536000; includeSubDomains`) whenever `externalUrl` is https (always, outside `--allow-insecure-localhost`).
- [x] `Cache-Control: no-store` on `/api/*`, `/auth/*`, `/ws`.

### CSRF, Origin, redirects
- [x] `/ws` upgrade and `/auth/logout` require an exact `Origin` match with `externalUrl`; logout also needs the CSRF header, compared in constant time.
- [x] Login state is single-use; bad `state` and replayed codes are rejected (`oidc_flow::bad_state_is_rejected_before_any_exchange`, `code_cannot_be_replayed`); nonce and audience checked.
- [x] Open redirect: `return_to` accepts only same-origin absolute paths, no `//`, `\`, `://` and only visible ASCII (S5).
- [x] Header injection: CR/LF cannot reach `Location` (S5); impersonation headers refuse identities that are not valid header values (`impersonation::unrepresentable_identity_is_refused`).

### Isolation and authorization
- [x] Per-user backend, runtime and state root (S1). Cross-user WS frame isolation: `http_ws::ws_frames_filter_and_isolation`.
- [x] Shared-state commands denied by `BackendCmdV1::hosted_policy`.
- [x] Impersonation: SA can only `impersonate` users/groups; it has no reads of its own; empty groups is an explicit error state (`impersonation.rs`).
- [x] Audit: append-only, no edit/delete operation in the protocol (`audit_visibility::protocol_has_no_audit_edit_or_delete_operation`); file `0600`, dir `0700`.
- [x] Fail-safe: OIDC discovery failure means no listener (`failsafe.rs`).

### Dependency audit (`cargo audit`, 2026-09-26)

| Advisory | Crate | Reaches `fathom-server`? | Assessment |
|---|---|---|---|
| RUSTSEC-2026-0189 (high) | `rmcp 0.12.0` | Linked via `fathom` | Affects the Streamable HTTP server transport only; Fathom enables `server` + `transport-io` (stdio). Not reachable. Upgrade to `>=1.4` is a separate (breaking) change for the MCP feature. |
| RUSTSEC-2023-0071 (medium) | `rsa 0.9.10` | Via `openidconnect` | Marvin timing attack targets private-key decryption/signing. Fathom only verifies RS256 signatures with public keys. Not exploitable here; no fixed version exists. |
| RUSTSEC-2026-0258 | `h2 0.3.27` | No (server uses `h2 0.4`) | Desktop-only dependency path. |
| RUSTSEC-2026-0098/0099/0104 | `rustls-webpki 0.101.7` | No (server uses `0.103.x`) | Desktop-only dependency path. |
| Unmaintained warnings | `fxhash`, `paste`, `proc-macro-error`, `rustls-pemfile 1` | Transitive | Informational. |

`cargo deny` is installed but the repo has no `deny.toml`; adding a policy file is left to the
release work so as not to introduce a new CI gate while CI is disabled.

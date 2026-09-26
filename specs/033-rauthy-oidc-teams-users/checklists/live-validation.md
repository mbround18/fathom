# Live validation: 033-rauthy-oidc-teams-users (T023, quickstart section 2)

Run 2026-09-26 against the lab cluster `kind-fathom-oidc-rauthy`. The issuer was `https://auth.localtest.me/auth/v1/` and the client `fathom-desktop`. The API server is configured with `k8s/kind-oidc-apiserver` (AuthenticationConfiguration, prefixes `oidc:`). Only the lab's dev users and password were used.

**Harness.** The real backend ran headless via `examples/live_driver.rs`, with in-memory secrets. A fake `xdg-open` first on PATH captured the authorize URL that Fathom opens. Headless Chromium (Playwright) then acted as the system browser: it filled Rauthy's real login form and followed the redirect to Fathom's loopback listener. The issuer CA came from the lab's `certificateAuthority` (the `ca_bundle_path`).

**Section 1 (sign-in), exercised as a prerequisite:**
- The authorize URL had `redirect_uri=http://127.0.0.1:<port>/callback` and PKCE `S256`. Rauthy redirected back with `code` and `state`, and the loopback page said "Signed in to Fathom. You can close this tab."
- Stages went `Awaiting` → `Validating` → `Done`.
- The identity was `alice`, groups `[fathom-viewers]`, state `SignedIn`.
- The access token lifetime is 60s in the lab. Over 80s the session refreshed silently twice (`Refreshing` → `SignedIn`, `expires_at` +40s then +80s) with no `OidcSessionEnded`, and API calls kept working.
- Sign-out cleared the identity.

**Section 2 (cluster login):**

| Check | alice (`fathom-viewers`) | bob (`fathom-editors`) |
|-------|--------------------------|-------------------------|
| `SetContextAuthMode oidc`, then connect | Connected; pods listed via OIDC | Connected |
| `CapabilitiesResolved` (SelfSubjectRulesReview) | 12 rules, sub `ZhAR2q...` | 25 rules, sub `QLY44T...` |
| list pods in `demo` | allowed | allowed |
| delete pods in `demo` | **denied** (`no RBAC rule grants 'delete' on pods`) | **allowed** (binding `oidc:fathom-editors` → `edit`) |
| list or delete nodes | denied | denied |
| after 2 silent refreshes | still allowed to list | still allowed to list |

The kubeconfig admin identity could delete anything. The OIDC identities got exactly the RBAC of their `oidc:`-prefixed groups, so the API server authenticated the Rauthy ID token, not the kubeconfig certificate. **Pass.**

**Break the issuer config.** The OIDC mode was pointed at an API server that does not trust the issuer (the throwaway kind `fathom-fed-1`). The reconnect failed with the verbatim API server error, and the capability check reported the same 401, unverified:
`ApiError: Unauthorized: Unauthorized (ErrorResponse { status: "Failure", message: "Unauthorized", reason: "Unauthorized", code: 401 })`
Nothing fell back to that context's admin kubeconfig certificate. After sign-out, switching to the OIDC context failed with `OIDC session for issuer "rauthy" ended or was never started; sign in again (kubeconfig credentials are not used for OIDC contexts)`. **Pass.**

**Regression.** Switching the context back to `kubeconfig` mode restored the admin identity (delete allowed). No JWT-looking string appears in the event stream or the backend log.

**Observation (not changed).** On a context switch, `ContextStatusUpdated` reports `Connected` as soon as the client is built, before the first API round trip. On an OIDC context the API server rejects, the switch then fails, via `CommandFinished` plus an error notification carrying the verbatim 401. The status briefly reads `Connected` even so. This is pre-existing and not OIDC-specific; it is worth a follow-up so that status reflects the first authenticated call.

Not exercised:
- the device-code fallback (blocked loopback);
- the Settings → Identity UI in the real window (no WebDriver on this host; the fake-backend e2e covers it).

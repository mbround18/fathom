# Research: Hosted Server Mode

Grounded in: `src/bridge` (pump/hub/`BackendLink`, `spawn_backend`), `tools/devbridge`
(`auth.rs` origin+token, `guard.rs` write guard, `server.rs` frames, `main.rs` context pinning),
`ui/src-tauri`, `ui/src/protocol/{transport,ws-transport}.ts`, `src/backend` (`state.rs`,
`kube.rs`, `backend.rs::init` and the audit call at `backend.rs:212`), `src/backend/audit_log.rs`,
`src/protocol/v1/{mutation,audit,bridge_types}.rs`, `src/backend/rbac_access.rs` (spec 018),
`src/backend/mcp` (spec 015), `k8s/`. Note: `Cargo.lock` is not tracked in git (dependency
versions below are pinned by requirement, and T003 records `cargo tree -d` results).

## R1: Server crate layout

**Decision**: New workspace member `server/` (`fathom-server`, lib + bin), peer of `ui/src-tauri` and `tools/devbridge`, depending on `fathom = { path = "..", default-features = false }`.

**Rationale**: devbridge is a dev-only fence (loopback, shared token, sandbox namespace, MCP off) and lives under `tools/` on purpose. A production, network-facing shell must not inherit or weaken those fences (FR-018). A separate crate keeps the server's dependency surface (axum, openidconnect) out of the desktop build and out of devbridge.

**Alternatives**: extend devbridge (rejected: blurs dev/prod fences); `tools/`-style dir (rejected: misleading for a shipped artifact).

## R2: HTTP/WS stack

**Decision**: `axum` 0.8 with `ws`, on `tokio`; static UI via `tower-http::ServeDir`; TLS terminated by the operator's ingress (spec assumption), with `Secure` cookies and an `X-Forwarded-Proto`/configured external URL check. Optional in-process TLS is deferred.

**Rationale**: hyper, hyper-util, tower and tower-http are already in the graph through `kube`/`reqwest`; axum adds only a thin layer and gives routing, extractors, cookies and WS upgrade needed for `/auth/*`, `/api/session`, `/healthz`, `/ws`. devbridge hand-rolls `tokio-tungstenite` because it has one endpoint; the server needs an HTTP surface (redirects, cookies, static files), where hand-rolling would reimplement axum badly. T003 verifies axum's `tokio-tungstenite` resolves to the same 0.27 kube uses (pin axum patch version accordingly); fallback is `hyper` + `tokio-tungstenite` with the devbridge handshake pattern.

**Alternatives**: actix-web (new runtime/ecosystem), warp (less maintained), raw hyper (too much boilerplate).

## R3: OIDC client

**Decision**: `openidconnect` 4.x (stable) for discovery, PKCE auth-code, code exchange, ID-token validation (signature via JWKS, `iss`, `aud`, `exp`, `nonce`), refresh, and RP-initiated logout URL. Provide it a custom async HTTP client adapter backed by the repo's existing `reqwest` 0.13 so no second reqwest/TLS stack is pulled (T003 asserts via `cargo tree -d`); custom CA file supported for the kind overlay.

**Rationale**: FR-020 needs no provider-specific paths; a maintained, audited OIDC/JOSE implementation avoids hand-rolled JWT validation. Rauthy and Keycloak both expose standard discovery.

**Alternatives**: `oauth2` + `jsonwebtoken` by hand (more security-sensitive code to own); `openid` crate (less maintained).

## R4: Session store, cookie, CSRF, origin

**Decision**:
- In-memory `HashMap<SessionId, Session>` behind `tokio::sync::RwLock`, ids = 256-bit CSPRNG, base64url. Sweeper task every 30 s enforces absolute (default 8 h) and idle (default 30 min, refreshed by WS activity and API calls) lifetimes and sign-out.
- Cookie `__Host-fathom_session`: `HttpOnly; Secure; SameSite=Lax; Path=/`, no `Domain` (Lax, not Strict, because the IdP redirect back to `/auth/callback` is a cross-site top-level navigation). The pre-login state/nonce/PKCE verifier live in a short-lived (5 min) sealed `__Host-fathom_login` cookie plus server-side map keyed by `state`.
- CSRF: all state-changing HTTP (`POST /auth/logout`, etc.) require `X-Fathom-CSRF` equal to a per-session token delivered by `GET /api/session` (never in a cookie script can read the session from) AND an `Origin` (or `Sec-Fetch-Site: same-origin`) match against the configured external origin. GET routes are side-effect free.
- WebSocket: `/ws` upgrade requires valid session cookie, `Origin` exactly equals the external origin (contrast devbridge's localhost allow-list), and a `Sec-WebSocket-Protocol`-free handshake; the CSRF token is not needed because Origin is enforced and cookies are SameSite. Reject with 403/401 before upgrade.
- Response headers: `Cache-Control: no-store` on `/api/*`, strict CSP (`default-src 'self'; connect-src 'self' wss:`), `X-Content-Type-Options`, `Referrer-Policy: no-referrer`, HSTS delegated to ingress but set when `Secure` proxy detected.

**Rationale**: Meets FR-010/011 and the edge cases (proxy, tabs). Multiple tabs share the cookie and therefore one session and one `UserBackend`; a second WS `subscribe` replaces the first subscriber exactly like the existing hub semantics, so N tabs must be supported: the hub is extended with fan-out (see R7, T-lane B) or later tabs get a read-only mirror, decision: **fan-out hub with a cap of `maxTabsPerUser` (default 5)** so tabs do not duplicate cluster load.

## R5: Per-user kube client and token passthrough

**Decision**: Introduce `ClusterAccess` in `src/backend/access.rs` with two implementations: `KubeconfigAccess` (today's `load_kubeconfig` + `Client::try_from(Config)`, the default; desktop and devbridge unchanged) and, in `server/`, `HostedAccess { home: Config, identity: Identity, token: Arc<TokenCell>, model }`. Clients are built with `kube::Client::new(service, default_ns)` where `service` is the kube `Config`'s HTTP stack plus a tower layer that sets `Authorization: Bearer <current token>` per request from the `TokenCell` (pass-through) or the service account token plus `Impersonate-User`/`Impersonate-Group` headers (impersonation). API server URL and CA come from the in-cluster environment (`Config::incluster()`), never a kubeconfig.

Token used in pass-through: the **ID token** by default (Kubernetes OIDC authenticator validates ID tokens; `aud` must equal the API server's `--oidc-client-id`), with a config switch `kubeToken: id|access` for providers issuing JWT access tokens accepted by the API server.

**Impersonation opt-in**: Fathom SA gets ClusterRole `impersonate` on `users`, `groups` (and `userextras` only if configured), no other verbs. User name = `usernamePrefix + claim(usernameClaim)`, groups = `groupsPrefix + claim(groupsClaim)` (each configurable, FR-020). Optionally restrict with `resourceNames` from `impersonation.allowedGroups`. Chart renders that ClusterRole only in impersonation mode.

**Rationale**: `Client::new` with a layer is the supported kube-rs extension point and covers HTTP, watch, exec and port-forward (all go through the same service, including WS upgrades), so no per-feature token plumbing.

## R6: Token refresh during long-lived watches

**Decision**: Each session owns a `TokenCell` (`Arc<RwLock<SecretString>>` plus `expires_at`). A refresh task per session renews the tokens at 75% of remaining ID-token lifetime (min 20 s spacing) via the refresh-token grant, using rotating-refresh-token semantics (store the new one). On success it swaps the cell; the auth layer picks it up on the next request, so watcher reconnects, log follow restarts, exec and port-forward *new* connections use the fresh token. Established streams: the API server authenticates at connect time, and Fathom additionally **recycles** watch/stream connections older than `streamRecycleMinutes` (default 4) so revocation is bounded (SC-005: <= 5 min) and lapsed users cannot hold a stream indefinitely; recycle is invisible to the UI (existing watchers already resume from `resourceVersion`). Exec shells and port forwards are not recycled mid-use (would break the user's work) but are terminated when the session ends or refresh fails permanently, with a reconnectable state.

**Failure handling**: refresh returns `invalid_grant` (revoked/expired) or IdP unreachable beyond `refreshGraceSeconds` (default 120) => session ended: emit a terminal `SessionEnded` system event, abort the `UserBackend`, close WS with code 4401. IdP outage otherwise lets the user continue until token expiry (spec edge case); never falls open.

**Revocation latency**: bounded by min(ID-token lifetime, refresh cadence, recycle interval); documented recommendation: IdP access/ID token lifetime <= 5 min, and `revalidateSeconds` (default 60) forces a `userinfo`/refresh probe to notice revoked sessions.

## R7: Per-user Backend isolation and caps

**Decision**: One `UserBackend` per *session-identity* (keyed by `sub`, so tabs and reconnects share it; a new sign-in of the same `sub` reuses the live instance if the old session ended <= `reuseWindow`). Each has: its own `BackendLink`, `Hub`, crossbeam channels, and a `spawn_backend_with(access, sink, args)` thread + Tokio runtime (this is exactly `backend::init` today, which already builds its own `Runtime` and `BackendState` per call, so isolation is by construction). Worker threads per runtime capped (`worker_threads = 2`) to bound 25-user cost.

**Caps (limits.rs)**: `maxSessions` (default 50 global), `maxSessionsPerUser` (3), `maxWatchesPerUser` (30), `maxStreamsPerUser` (exec+logs+port-forward, 10), `maxTabsPerUser` (5). Watches/streams are counted in a `LimitedGuard` wrapper installed in the per-user command path (`filter.rs` sees `StartLog/Exec/PortForward` and watch-starting commands); overflow returns a `BridgeError::Forbidden("limit: ...")` mapped by the UI to a "limit reached" banner (FR-008). Idle parking: no WS subscribers for `idleParkMinutes` => backend told to stop pollers (existing `abort_all`) and dropped after `sessionIdle`.

**Backend code assuming a single global user/context that must change**:
1. `backend/kube.rs::connect_client_for_current_context`, `context_status.rs`, `fleet.rs` (kubeconfig-based, multi-context) -> behind `ClusterAccess`; hosted: single home context named `in-cluster`, `SetContext` other than home refused.
2. `backend/persistence.rs` (`~/.fathom` via `dirs::home_dir()`) -> injectable state root (`FATHOM_STATE_DIR/<sha256(sub)[..16]>/`) so port-forward configs, credentials, agent conversations never collide; default unchanged.
3. `audit_log.rs` global file + `COUNTER` static -> `AuditSink` seam (R10).
4. `events::take_cmd_errors()` (used by `backend.rs` to compute `ok`/`error`) must be verified per-runtime/thread-local, not process-global (T-audit of statics; expected thread-local, verify).
5. `ExecSessionState` "one active application-wide" is per-`BackendState`, so per-user; fine.
6. `mcp` (stdio) -> `Args::disable_mcp = true` (existing knob).
7. `metrics_history` clear-on-startup in `init` would wipe shared state: skipped in hosted (access seam flag).
8. Generic `Client::try_default()` fallbacks in `connect_client_for_current_context` must be unreachable in hosted (fail closed).

**Alternatives**: single shared Backend with per-command identity (rejected: pervasive rewrite, easy to leak state); process per user (rejected: memory).

## R8: Hiding local-only features in hosted mode

**Decision**: Defense in depth. (1) Server: `filter.rs` allow-lists commands by `MutationTarget`/variant; everything classified `Local` (kind provisioning, `SetContext` away from home, kubeconfig refresh/`RefreshContexts`, MCP enable/disable, fleet, agent-config local paths) returns `BridgeError::Forbidden`. A table test asserts every `BackendCmdV1` variant is explicitly classified hosted-allowed/denied (exhaustive match, no wildcard, like the existing mutation classifier). (2) Protocol: `SubscriptionInfo.mode = "hosted"` plus `capabilities` list; (3) UI gates nav/settings entries on `mode`; UI hiding is UX, the filter is the boundary.

## R9: Identity claims and group mapping (FR-020, edge cases)

**Decision**: Config `usernameClaim` (default `preferred_username`, fallback `email`, then `sub`), `groupsClaim` (default `groups`), prefixes matching the kind API server flags (`oidc:`). Missing groups claim with impersonation => explicit "no groups" error state; too-large groups (IdP omitted) => user sees explanation and (Keycloak/Rauthy) hint to use a group mapper; we do not resolve via userinfo unless `groupsFromUserinfo: true`.

## R10: Audit actor attribution

**Decision**: Additive `actor: Option<AuditActor { sub, username, groups? }>` on `AuditLogEntry` (`#[serde(default, skip_serializing_if = "Option::is_none")]`, ts-rs regenerated; desktop entries have `None`, older files still load: Principle VI). `BackendState` gains `identity: Option<Identity>`; `backend.rs:212` passes it to `audit_log::record`, which delegates to an `AuditSink` trait: `LocalFileSink` (today) or `HostedAuditSink` (shared append-only JSONL under state dir, mutex-guarded, also emitted as structured `tracing` for log shippers). Impersonation mode records the *real* user (identity from the session, not the SA). Viewing: non-admin users get only `actor.sub == self`; `auditViewerGroups` config allows an operator group to see all. No edit/delete op exists in the protocol; file mode 0600, dir owned by the non-root UID. Retention bound reused (`AUDIT_LOG_RETENTION`; hosted default larger via config).

## R11: Container image and static UI

**Decision**: `deploy/Dockerfile` multi-stage: (1) node stage `pnpm -C ui build` with `VITE_TRANSPORT=hosted`; (2) rust stage builds `fathom-server` for `*-unknown-linux-musl` (rustls + ring, no OpenSSL; T-verify `reqwest` `default-tls` resolves to rustls in 0.13); (3) `gcr.io/distroless/static-debian12:nonroot`, UID 65532, read-only root FS, UI at `/ui`, binary at `/fathom-server`, state at `/var/lib/fathom` (volume). UI is *served from disk* (`ServeDir`), not embedded: keeps the cargo build independent of the pnpm build, lets Playwright and Helm test the same image layout, and avoids a giant binary. Embedding (rust-embed) stays a possible later optimization.

## R12: Helm chart layout

**Decision**: `deploy/helm/fathom/` with `Chart.yaml`, `values.yaml`, `values.schema.json` (required `oidc.issuer`, `oidc.clientId`, `externalUrl`; secret via `existingSecret` or `clientSecret`), templates: `deployment`, `service`, `ingress` (optional, TLS), `serviceaccount`, `clusterrole-impersonate` + binding (only when `accessModel=impersonation`), `clusterrole-home-min` (Fathom's own SA needs only what its startup needs: none for pass-through beyond `get` on its namespace's self; no cluster-wide grants), `secret`, `pdb` (off), `networkpolicy` (optional), `NOTES.txt`. Single command: `helm install fathom deploy/helm/fathom -n fathom --create-namespace -f my-values.yaml`. Secure defaults: non-root, RO FS, dropped caps, seccomp RuntimeDefault, `replicaCount: 1` (schema rejects >1 in v1). `helm uninstall` leaves nothing cluster-scoped (all cluster-scoped objects are chart-owned and removed).

## R13: kind overlay with Rauthy (primary) and Keycloak (second)

**Decision**: `k8s/kind-oidc.yaml` kind config with `kubeadmConfigPatches` `ClusterConfiguration.apiServer.extraArgs`: `oidc-issuer-url`, `oidc-client-id=fathom`, `oidc-username-claim`, `oidc-groups-claim`, `oidc-username-prefix=oidc:`, `oidc-groups-prefix=oidc:`, `oidc-ca-file=/etc/ca/ca.crt`, and `extraMounts`/`extraVolumes` for the CA; `extraPortMappings` 80/443 for ingress. `cluster.sh up-hosted`: generates a throwaway CA + leaf certs (openssl), creates the cluster from `kind-oidc.yaml`, installs ingress-nginx, deploys Rauthy and Keycloak (kustomize, `overlays/hosted`), seeds realms/users/clients (Keycloak realm import JSON; Rauthy via bootstrap env/API job), `docker build` + `kind load` the image, helm-installs Fathom pointed at Rauthy (Keycloak as a second values file). Users: `alice` (group `fathom-viewers` -> `view` in `demo`), `bob` (`fathom-editors` -> `edit` in `demo`), `carol` (no group). Hostnames `auth.localtest.me`, `keycloak.localtest.me`, `fathom.localtest.me`. Name resolution is the hard part: pods and the API server static pod must resolve those names to the ingress: CoreDNS `rewrite` to the ingress controller Service for pods, and node `/etc/hosts` entries injected with `docker exec` for the apiserver (kubelet static pod hostNetwork) (risk R-4). API-server flags are fixed at cluster create, so Rauthy and Keycloak get **separate clusters** (`fathom-oidc-rauthy`, `fathom-oidc-keycloak`) or two clients with one issuer each is impossible; decision: two kind clusters, same overlay, provider chosen by `PROVIDER=rauthy|keycloak`.

## R14: E2E login tests

**Decision**: `ui/tests/hosted/` with its own Playwright config running per provider (`PROVIDER` env). Scenarios: sign-in (PKCE code flow through the provider's real login page), silent renewal (short token lifetime configured in the provider; assert no UI interruption and a changed token via server-side test hook counter at `/api/session` `refreshCount`, non-secret), sign-out (session invalid, WS closed), expired (force by provider lifetime/`FATHOM_TEST_CLOCK_SKEW` off in prod builds; prefer a real short lifetime), revoked (admin API disables the user/kills session, assert live view stops within the bound), pass-through RBAC: alice cannot delete in `demo`, bob can, carol sees no-access; shell/port-forward permission checks. No test-only bypass exists in the server binary (`cfg(test)`/feature `e2e-hooks` gates only the non-secret counter).

## Risks

- R-1 Constitution amendment gate (MINOR bump) blocks implementation; T001.
- R-2 Per-user thread+runtime memory/CPU at 25 users; measure in T-polish; fallback `worker_threads=1`.
- R-3 Dependency duplication (axum/tokio-tungstenite/openidconnect/reqwest); T003 gate with explicit fallbacks.
- R-4 kind DNS/TLS for issuer reachable from browser, pods and apiserver simultaneously.
- R-5 Two provider clusters double e2e time; run providers as parallel CI jobs.
- R-6 Global statics in backend (`take_cmd_errors`, counters) may cross-contaminate per-user backends; audit in T-B.
- R-7 Long-lived established exec/port-forward streams outlive revocation until recycle/refresh failure ends the session; documented and bounded by session termination.
- R-8 Group claim size/absence differs per IdP; explicit UI explanation, documented mappers.
- R-9 Cookie-only auth for WS means proxies must pass `Upgrade` and preserve `Origin`/`Host`; documented in quickstart; a health check for proxy misconfiguration on `/api/session`.
- R-10 ID-token-as-bearer requires API server OIDC; managed clusters without it must use impersonation (documented, not auto-detected per spec).

# Tasks: Hosted Server Mode (In-Cluster Web App with OIDC Login)

**Input**: `/specs/031-hosted-server-mode/` (plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md)

**Tests**: Included (constitution Principle III, spec FR-021). Tests precede or accompany the code they cover.

**Organization**: Setup -> Foundational -> US1 (sign-in + RBAC) -> US2 (Helm install + kind overlay) -> US3 (audit attribution) -> Polish.

## Format: `[ID] [P?] [Lane] [Story] Description`

- **[P]**: files disjoint from every other task in the same batch, safe to run in parallel.
- **Lane A-E** (see plan.md "Parallel Implementation Lanes"): separate agents. Max 5 concurrent agents. **Only one agent may run a `cargo` or `pnpm` build/test at a time** (coordinate through the integration owner); other agents edit files and wait for their turn to build.
- Foundational (Phase 2) merges before lanes fan out.

## Phase 1: Setup

- [ ] T001 [Gate] Amend the constitution to v1.1.0 (MINOR) via `/speckit-constitution`: replace the "browser-hosted web build is out of scope" sentence in `.specify/memory/constitution.md` with a "Hosted server (Linux container, amd64/arm64)" tier and keep mobile out of scope; include the Sync Impact Report. **Blocks all implementation tasks.**
- [ ] T002 Create `server/Cargo.toml` (crate `fathom-server`, lib + bin, `fathom` with `default-features = false`) and add `"server"` to `members` in `/Cargo.toml`; add empty `server/src/{main,lib}.rs`.
- [ ] T003 Record dependency resolution in `specs/031-hosted-server-mode/research.md` (append "R15 Verified deps"): add `axum` 0.8 (`ws`), `openidconnect` 4, `tower-http`, `secrecy`, `rand`; run `cargo tree -d -p fathom-server` and confirm a single `tokio-tungstenite` (0.27) and `reqwest` (0.13, rustls); if duplication appears apply the documented fallbacks (hyper + tokio-tungstenite handshake; custom `openidconnect` HTTP client).
- [ ] T004 [P] Create empty skeletons `deploy/Dockerfile`, `deploy/helm/fathom/Chart.yaml`, `k8s/kind-oidc.yaml`, `k8s/overlays/hosted/kustomization.yaml` with header comments.
- [ ] T005 [P] Add `Makefile.toml` tasks `server-build`, `server-test`, `hosted-image`, `hosted-e2e` in `/Makefile.toml`.
- [ ] T006 [P] Add `hosted` to `TransportKind` in `ui/src/protocol/transport.ts` (folded at build time by `VITE_TRANSPORT=hosted`, lazy import of `./hosted-transport`).

**Checkpoint**: workspace resolves, deps deduped, amendment merged.

## Phase 2: Foundational (Blocking)

**CRITICAL**: no user-story task starts until this phase merges.

- [ ] T007 Protocol additions in `src/protocol/v1/bridge_types.rs` (`ShellMode`, `PublicIdentity`, `PublicLimits`, optional fields on `SubscriptionInfo`) and `src/protocol/v1/event.rs` (`SystemEvent::SessionEnded`, `LimitReached`); re-export from `src/protocol/v1/mod.rs`; regenerate ts-rs bindings under `ui/src/protocol/generated/`.
- [ ] T008 Serde round-trip tests for every new/changed type (incl. a legacy `SubscriptionInfo` without new fields deserializing), in the existing protocol test module.
- [ ] T009 `ClusterAccess` seam: `src/backend/access.rs` with trait (`build_client(&self, context) -> Client`, `home_context()`, `allows_local_features()`, `state_root()`), default `KubeconfigAccess` reproducing today's `connect_client_for_current_context`; route `src/backend/kube.rs` through it; `BackendState` holds `Arc<dyn ClusterAccess>`. Existing tests must pass unchanged.
- [ ] T010 [P] `AuditSink` seam in `src/backend/audit_log.rs` (`LocalFileSink` default = current behavior, `record(..., actor)` signature with `Option<AuditActor>`); `BackendState.identity: Option<Identity>`; update the call at `src/backend.rs:212`. Add `actor` (serde default/skip-if-none) to `AuditLogEntry` in `src/protocol/v1/audit.rs`; regenerate binding; legacy-file load test.
- [ ] T011 [P] Injectable state root in `src/backend/persistence.rs` (default `~/.fathom`; override via `ClusterAccess::state_root`); test that default paths are unchanged.
- [ ] T012 `spawn_backend_with(access, sink, args)` in `src/bridge/mod.rs` (existing `spawn_backend` delegates with defaults); skip `clear_metrics_history` when access is hosted.
- [ ] T013 Audit backend statics for cross-user leakage (`events::take_cmd_errors`, counters, any `static`/`lazy` state) via grep across `src/backend`; make thread-local or per-state where needed; add a test running two backends concurrently asserting error/`ok` isolation.
- [ ] T014 Verify FR-018: run the existing `cargo test`, `cargo clippy -- -D warnings`, devbridge and `ui/src-tauri` builds unchanged.

**Checkpoint**: seams merged with default behavior identical; desktop and devbridge green.

## Phase 3: User Story 1 - Sign in and see/do only what RBAC allows (P1) - MVP

**Goal**: OIDC sign-in, per-user isolated backend, per-user kube identity, hidden local features, sign-out, limits, refresh.

**Independent Test**: two users with different RBAC, each allowed/refused by the API server itself, incl. shell and port-forward; sessions isolated; sign-out stops live views.

### Tests first (fail before implementation)

- [ ] T015 [P] [A] [US1] `server/src/csrf.rs` tests+impl: origin exact-match, `Sec-Fetch-Site`, CSRF header compare (constant time), return-path open-redirect rejection. Files: `server/src/csrf.rs`.
- [ ] T016 [P] [A] [US1] `server/src/session.rs` tests+impl: id generation, idle/absolute expiry (tokio paused time), sweeper, sign-out, per-user session cap. Files: `server/src/session.rs`.
- [ ] T017 [P] [B] [US1] `server/src/filter.rs` tests+impl: exhaustive table classifying every `BackendCmdV1` variant hosted-allowed/denied (denies kind, kubeconfig, contexts, MCP, fleet, local paths, `SetContext` != home). Files: `server/src/filter.rs`, `src/protocol/v1/mutation.rs` (helper only).
- [ ] T018 [P] [B] [US1] `server/src/limits.rs` tests+impl: per-user/global counters, RAII guards, `LimitReached` reporting. Files: `server/src/limits.rs`.
- [ ] T019 [P] [B] [US1] `server/src/kubeauth.rs` tests+impl: `TokenCell`, tower auth layer setting bearer per request, impersonation headers (user + groups), `Config::incluster()` builder, token never in `Debug`/logs (secrecy). Files: `server/src/kubeauth.rs`.
- [ ] T020 [P] [A] [US1] `server/src/config.rs` tests+impl: parse/validate `HostedConfig` (https external URL, claim/prefix defaults, exit codes 2/3, no bind before discovery). Files: `server/src/config.rs`.

### Implementation

- [ ] T021 [A] [US1] `server/src/oidc.rs`: discovery at startup (fail closed), auth-code + PKCE `S256`, state/nonce, ID-token validation, claim -> `Identity` mapping (configurable username/groups claims, missing-groups error), refresh-token grant with rotation, RP-initiated logout URL, custom CA, reqwest-backed HTTP client. Depends on T020. Files: `server/src/oidc.rs`.
- [ ] T022 [A] [US1] Mock-IdP integration tests in `server/tests/oidc_flow.rs` (local JWKS server): happy path, bad `state`, bad `nonce`, wrong `aud`, expired token, refresh success, `invalid_grant`. Depends on T021.
- [ ] T023 [B] [US1] `server/src/tenant.rs`: `UserBackend` create/reuse by `user_key`, `HostedAccess` `ClusterAccess` impl (home `in-cluster`, refuses other contexts, no kubeconfig, no `try_default` fallback), `spawn_backend_with`, runtime `worker_threads=2`, idle park, teardown/abort on session end, fan-out hub (max tabs). Depends on T009, T012, T017-T019.
- [ ] T024 [B] [US1] Refresh task + stream recycle in `server/src/tenant.rs` (`server/src/refresh.rs`): 75%-lifetime renewal swaps `TokenCell`; permanent failure or grace exceeded emits `SessionEnded` and aborts backend; watch/log stream recycle every `streamRecycleMinutes`; `revalidateSeconds` probe. Tests with paused time and mock IdP. Depends on T021, T023.
- [ ] T025 [A] [US1] `server/src/http.rs`: axum router (`/auth/login`, `/auth/callback`, `/auth/logout`, `/api/session`, `/healthz`, `/readyz`, static `ServeDir` with SPA fallback), cookie flags, security headers, CSP, `Cache-Control: no-store` on `/api`. Depends on T015, T016, T021.
- [ ] T026 [A] [US1] `server/src/ws.rs`: `/ws` upgrade (session + exact Origin, pre-upgrade 401/403/429), reuse `DevBridgeClientMsg/ServerMsg` frames, per-session `send` through filter + limits, `subscribe` attaching to the user's hub with `mode/identity/limits` in `SubscriptionInfo`, close codes 4401/4429/4403. Depends on T023, T025.
- [ ] T027 [A] [US1] `server/src/main.rs`: config load, discovery-or-exit, graceful shutdown, `install_crypto_provider`, `init_tracing` with token redaction test (FR-017). Depends on T025, T026.
- [ ] T028 [A] [US1] Server integration tests `server/tests/http_ws.rs`: unauthenticated redirect, CSRF/origin rejection on logout and `/ws`, cookie attributes (`HttpOnly`, `Secure`, `SameSite=Lax`, `__Host-`), two sessions isolated (events never cross), sign-out closes WS and ends views, `/readyz` states. Depends on T027.
- [ ] T029 [P] [C] [US1] `ui/src/protocol/hosted-transport.ts` + Vitest `ui/tests/unit/hosted-transport.test.ts`: same-origin `wss`, cookie auth, `GET /api/session` bootstrap + CSRF, 401/4401 -> sign-in redirect preserving return path, reconnect with backoff, no token in URL.
- [ ] T030 [P] [C] [US1] `ui/src/components/hosted/` (`SignInGate.tsx`, `SessionExpired.tsx`, `NoAccess.tsx`, `LimitReached.tsx`, `UserMenu.tsx` with sign-out): mobile-first, shadcn primitives, verbatim refusal messages.
- [ ] T031 [C] [US1] Store wiring: apply `mode/identity/limits` in `ui/src/store/handlers/system.ts` and `ui/src/store/index.ts`; `SessionEnded`/`LimitReached` handling; hide kind provisioning, kubeconfig/context switcher, MCP and fleet entries when `mode === 'hosted'` (nav in `ui/src/resources/nav.ts`, settings panels). Depends on T007, T029, T030.
- [ ] T032 [C] [US1] Fake-backend Playwright specs `ui/tests/e2e/hosted-signin.spec.ts`, `hosted-no-access.spec.ts`, `hosted-limits.spec.ts`, `hosted-hidden-local.spec.ts`, `hosted-signout.spec.ts` (scripted events/commands, Principle III). Depends on T031.
- [ ] T033 [B] [US1] Wire zero-permission detection: reuse spec 018 self-access (`RefreshRbacSelfAccess`) on first subscribe in hosted mode to emit the no-access state within 3 s (SC-008) and use the same permission-aware gating (FR-007); tests in `src/backend/rbac_access.rs`. Depends on T023.
- [ ] T034 [E] [US1] `k8s/kind-oidc.yaml` + `k8s/cluster.sh` `up-hosted`/`down-hosted` (CA gen, kind create with apiServer OIDC extraArgs and CA mount, ingress-nginx addon `k8s/addons/ingress-nginx/`, CoreDNS rewrite, node hosts injection); `PROVIDER=rauthy|keycloak`, `CLUSTER` env. Files: `k8s/kind-oidc.yaml`, `k8s/cluster.sh`, `k8s/addons/ingress-nginx/**`.
- [ ] T035 [P] [E] [US1] `k8s/addons/rauthy/**` (deployment, bootstrap job creating client `fathom` with PKCE, groups claim, users alice/bob/carol, short-lifetime test client settings).
- [ ] T036 [P] [E] [US1] `k8s/addons/keycloak/**` (deployment, realm import JSON with the same users/groups/mappers/client, short token lifetimes).
- [ ] T037 [P] [E] [US1] `k8s/samples/hosted-rbac.yaml`: namespace `demo`, ClusterRoleBindings/RoleBindings for `oidc:fathom-viewers` (view) and `oidc:fathom-editors` (edit); `k8s/overlays/hosted/kustomization.yaml` composing addons and samples.
- [ ] T038 [E] [US1] Live login suite `ui/tests/hosted/{auth.spec.ts,rbac.spec.ts,exec-portforward.spec.ts,helpers.ts}` + `ui/playwright.hosted.config.ts` + `e2e:hosted` script in `ui/package.json`: sign-in via real provider page (PKCE), silent renewal (short lifetime, `refreshCount` advances, no UI interruption), sign-out, expired, revoked (provider admin API, views end <= 5 min), alice refused / bob allowed, carol no-access, exec and port-forward per RBAC. Runs per `PROVIDER`. Depends on T027, T031, T034-T037, T045.

**Checkpoint**: US1 verified end to end on kind with Rauthy (SC-002, SC-005, SC-008) before fanning out further.

## Phase 4: User Story 2 - Helm install and kind overlay (P1)

**Goal**: one-command install, secure defaults, fail-visible on bad OIDC, impersonation opt-in, upgrade/uninstall cleanliness, test overlay with two providers.

**Independent Test**: clean kind cluster -> `k8s/cluster.sh up-hosted` -> sign in as sample users; invalid OIDC settings -> visible failure.

- [ ] T039 [P] [D] [US2] `deploy/Dockerfile`: multi-stage (node build with `VITE_TRANSPORT=hosted`, musl Rust build, `distroless/static-debian12:nonroot`), UID 65532, `/ui`, `/fathom-server`, healthcheck-free (probes in chart); `.dockerignore`. Verify rustls-only (no OpenSSL) via `ldd`/`cargo tree`.
- [ ] T040 [P] [D] [US2] Helm chart core: `deploy/helm/fathom/{Chart.yaml,values.yaml,values.schema.json,templates/{_helpers.tpl,deployment.yaml,service.yaml,serviceaccount.yaml,secret.yaml,ingress.yaml,NOTES.txt}}` per contracts/hosted-config.md; required values enforced by schema; `replicaCount` max 1; secure pod/container securityContext.
- [ ] T041 [D] [US2] Access-model RBAC templates `templates/{clusterrole-impersonate.yaml,clusterrolebinding-impersonate.yaml}` rendered only when `accessModel=impersonation` (verbs `impersonate` on `users`,`groups` only, optional `resourceNames`); probes (`/healthz`, `/readyz`), optional `networkpolicy.yaml`, `pdb.yaml` off by default. Depends on T040.
- [ ] T042 [D] [US2] Chart tests: `helm lint`, `helm template` golden checks in `deploy/helm/fathom/ci/` for both models and `kubeconform`; script asserting no cluster-scoped object is rendered in pass-through mode and that uninstall leaves none. Depends on T041.
- [ ] T043 [D] [US2] Fail-closed verification: server integration test `server/tests/failsafe.rs` (missing/invalid/unreachable issuer => exit 2/3, no listener) and a chart-level e2e in `ui/tests/hosted/failsafe.spec.ts` (bad issuer => CrashLoop, `/readyz` never 200). Depends on T027.
- [ ] T044 [E] [US2] `k8s/overlays/hosted/values-rauthy.yaml`, `values-keycloak.yaml` and finish `cluster.sh up-hosted` (build+`kind load` image, `helm install`, wait, print credentials); document DNS/CA handling in `k8s/README.md`. Depends on T034-T037, T039-T040.
- [ ] T045 [E] [US2] Second-provider bring-up: run T038 suite with `PROVIDER=keycloak` on its own kind cluster; fix provider-specific config only in overlay/values (no provider code paths, FR-020). Depends on T044.
- [ ] T046 [P] [A] [US2] Impersonation implementation tests: `server/tests/impersonation.rs` (headers, prefixing, missing groups error state, SA has no user reads) and live case in `ui/tests/hosted/impersonation.spec.ts` using `values-impersonation.yaml`. Depends on T019, T041.
- [ ] T047 [D] [US2] Upgrade/uninstall e2e in `k8s/cluster.sh` (`helm upgrade`, `helm uninstall`, assert no leftover ClusterRoles/Bindings); CI workflow `.github/workflows/hosted.yml` running lint, template, image build, and per-provider live suite as parallel jobs.

**Checkpoint**: SC-001, SC-004, SC-009, SC-010 verified on both providers.

## Phase 5: User Story 3 - Audit attributes each change to the real user (P2)

**Goal**: `actor` on every hosted audit entry in both models; restricted visibility; tamper resistance.

**Independent Test**: two users mutate; each entry names the correct user; alice sees only her entries; desktop unchanged.

- [ ] T048 [P] [B] [US3] `server/src/audit.rs`: `HostedAuditSink` (append-only JSONL, single writer mutex, 0600, structured tracing mirror, retention config) with tests incl. concurrent appends from two backends and corruption tolerance. Depends on T010.
- [ ] T049 [B] [US3] Attach `Identity`-derived `AuditActor` at backend creation in `server/src/tenant.rs` and thread it into `audit_log::record`; impersonation mode still records the real user; refused mutations follow existing spec 026 outcome rules. Depends on T023, T048.
- [ ] T050 [B] [US3] Hosted audit read path: filter entries by `actor.sub` unless a group in `audit_viewer_groups`; no edit/delete op exists (assert protocol has none); tests in `server/tests/audit_visibility.rs`. Depends on T049.
- [ ] T051 [P] [C] [US3] UI: show `actor` column/field in the audit viewer (`ui/src/components/**` audit view + `ui/src/store/auditLogFilter.ts` filter by actor), hidden when absent (desktop); Vitest + `ui/tests/e2e/hosted-audit-actor.spec.ts`. Depends on T007, T010.
- [ ] T052 [E] [US3] Live audit test `ui/tests/hosted/audit.spec.ts` (bob mutates, alice does not; entry names bob in both models; alice sees none of bob's; refused attempt per rules) for both providers. Depends on T049-T051, T044.
- [ ] T053 [B] [US3] Desktop regression test: `audit_log` default sink writes entries with `actor: None` and legacy `audit-log.json` still loads (Principle VI, FR-018/SC-003 scope). Depends on T010.

**Checkpoint**: SC-003 verified; all user stories independently functional.

## Phase 6: Polish and Cross-Cutting

- [ ] T054 [P] Load/soak test: 25 simulated users (`server/tests/soak.rs` or a script in `scripts/`) browsing and watching; assert isolation, no cross-user events, memory/CPU budget from plan (SC-006); tune `worker_threads`, idle park.
- [ ] T055 [P] Security review pass: token redaction sweep (`grep` for `Debug` on token types, tracing fields), cookie/CSP headers checklist, open-redirect and header-injection tests, dependency audit (`cargo deny`/`cargo audit`); run `/security-review`.
- [ ] T056 [P] Docs: `docs/HOSTED.md` (operator guide: OIDC setup for Rauthy/Keycloak/Dex/Entra/Okta claim tables, group mappers, ingress WebSocket/Origin notes, managed-cluster impersonation guidance), update `README.md` and `MVP.md`, `k8s/README.md`.
- [ ] T057 Run full gates once, serially: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (workspace), binding drift check, `pnpm -C ui lint typecheck test e2e`, `pnpm -C ui e2e:hosted` for both providers; execute quickstart.md end to end (SC-001 timing under 15 min).
- [ ] T058 `/speckit-analyze` cross-artifact consistency pass; resolve findings; mark tasks done.

## Dependencies and Execution Order

- Setup -> Foundational -> US1 -> {US2, US3} -> Polish. US2 and US3 depend on US1 for a working server but are independent of each other after US1's checkpoint.
- T001 blocks all. T002-T003 block server tasks. T007-T014 block all lanes.
- Within US1: tests (T015-T020) parallel; T021 after T020; T023 after T017-T019 + T009/T012; T025 after T015/T016/T021; T026 after T023/T025; T027 after T026; T038 last.

### Parallel fan-out (max 5 agents; one build at a time)

| Batch | Agents | Tasks |
|---|---|---|
| 1 (after Phase 2) | A, B, C, E | A: T015, T016, T020 -> T021; B: T017, T018, T019 -> T023; C: T029, T030; E: T034, T035, T036, T037 |
| 2 | A, B, C, D | A: T022, T025 -> T026 -> T027 -> T028; B: T024, T033; C: T031 -> T032; D: T039, T040 -> T041 -> T042 |
| 3 (US1 checkpoint) | E, then D | E: T038, T044, T045; D: T043, T047; A: T046 |
| 4 (US3) | B, C, E | B: T048 -> T049 -> T050, T053; C: T051; E: T052 |
| 5 (Polish) | any | T054, T055, T056 parallel; T057, T058 serial |

Rules for agents: touch only your lane's files (table in plan.md); Cargo.toml and `ui/package.json` edits are serialized through Setup and the integration owner; commit per lane, signed, no session links.

### MVP scope

Phases 1-3 (through T038 on Rauthy) deliver a working, secure sign-in with per-user RBAC. Phase 4 is required to ship (co-equal P1); US3 (P2) may follow.

## Notes

- UI hiding is UX; `server/src/filter.rs` and the cluster are the security boundary.
- No task adds a provider-specific code path; provider differences live in `k8s/` and values only.
- Desktop and devbridge defaults must stay byte-identical (T014, T053).

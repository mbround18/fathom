# Tasks: Rauthy OIDC Identity, Teams & Users

**Input**: specs/033-rauthy-oidc-teams-users/ (spec, plan, research, data-model, contracts/protocol.md, quickstart)
**Format**: `[ID] [P?] [Story] Description` ([P] = parallelizable, different files). Tests are mandatory (Constitution III). Fan out parallel-safe tasks across agents.

## Phase 1: Setup and lab validation

- [x] T001 Create `k8s/rauthy-clients/` kustomize: patch `rauthy-bootstrap` adding public client `fathom-desktop` (PKCE S256, loopback redirects, refresh_token + device_code, scopes incl. groups) and a read-only API key (Users:read, Groups:read); add `fathom-admins` group
- [x] T002 [P] Create `k8s/kind-oidc-apiserver/` (kubeadm patch or AuthenticationConfiguration, node hosts entry, CA file, prefixes `oidc:`) and notes
- [x] T003 [P] Add dependencies `openidconnect`, `keyring`, `arc-swap` to Cargo.toml with justification in research R2; confirm stable releases
- [x] T004 Lab spike against kind Rauthy: verify loopback redirect matching (R1), `aud` handling for API server (R3), Rauthy upstream group mapping (R4); record outcomes in research.md and pick fallbacks

## Phase 2: Foundational

- [x] T005 Add protocol types in `src/protocol/v1/oidc.rs`, `idp.rs`, `capabilities.rs` (contracts/protocol.md) and register variants in cmd/event enums
- [x] T006 Round-trip serialization tests for every new variant; regenerate ts bindings; CI drift check passes
- [x] T007 [P] Additive config: `oidc` key (issuer configs, context auth modes, role map) in `src/config.rs` with defaults-on-invalid and original-preserved tests
- [x] T008 [P] Keyring wrapper `src/backend/oidc/store.rs` with in-memory fallback and tests (fake backend)
- [x] T009 [P] Mock OIDC provider test harness (discovery, authorize, token, device, jwks, rotating keys) under `tests/support/`
- [x] T010 Redaction test: assert no token/secret string can appear in logs, protocol JSON or config (property test over all new types)

## Phase 3: US1 Sign in to Fathom (P1)

- [ ] T011 [US1] Tests first: discovery, PKCE flow, ID token validation failures (bad sig, iss, aud, exp, nonce, skew) in `src/backend/oidc/`
- [ ] T012 [US1] Implement `flow.rs` (browser + IPv4-only loopback listener on `127.0.0.1` (Rauthy does not match `[::1]`), 120 s timeout, cancel; drop empty `groups` claims, Rauthy omits/empties them) and `validate.rs` (JWKS cache/rotation)
- [ ] T013 [US1] Implement device-code flow and fallback selection
- [ ] T014 [US1] Implement `session.rs`: per-issuer refresh task off `exp`, ArcSwap token, rotating refresh token, `OidcSessionEnded`
- [ ] T015 [US1] Wire commands `OidcSaveIssuer/SignIn/CancelSignIn/SignOut` and events in `src/backend/cmd/`
- [ ] T016 [P] [US1] UI: `IssuerSettings`, `SignInPanel` (browser + device-code code display), `IdentityChip` (responsive) in `ui/src/features/identity/`
- [ ] T017 [US1] Vitest for identity state reducer; Playwright E2E with fake backend (sign in, cancel, expire, sign out)
- [ ] T018 [US1] Private-CA bundle support and clock-skew error message + tests

## Phase 4: US2 Cluster login with OIDC (P1)

- [ ] T019 [US2] Tests first: kube client uses current token, refreshes mid-session, never falls back after OIDC failure
- [ ] T020 [US2] Implement bearer layer in `src/backend/kube.rs` for OIDC-opted contexts; `SetContextAuthMode` command
- [ ] T021 [P] [US2] UI: per-context "authenticate with OIDC" control and verbatim-error panel linking to the 011 generator
- [ ] T022 [US2] Prefill 011 OIDC generator from discovery document (issuer, client id, claims, prefixes) + test; offer the kube-apiserver `AuthenticationConfiguration` (v1beta1) form as well as the legacy flags (T004: it is what the lab uses; supports multiple audiences)
- [ ] T023 [US2] Live validation per quickstart section 2 on kind (manual, record result)

## Phase 5: US5 RBAC-scoped UI (P2, before US3/US4)

- [ ] T024 [US5] Tests first: resolver from RulesReview, incomplete fallback to AccessReview, fail-closed, TTL and invalidation, lazy per namespace
- [ ] T025 [US5] Implement `src/backend/capabilities.rs` extending `rbac_access.rs`; commands `ResolveCapabilities`, `CheckCapability`
- [ ] T026 [US5] Refactor `src/backend/mcp/rbac_guard.rs` onto the resolver; keep 015 denial-message tests green
- [ ] T027 [P] [US5] UI `ui/src/lib/capabilities.ts`: `useCapability` hook and `<Gated>` wrapper (disabled + reason tooltip; hide only when kind unlistable)
- [ ] T028 [US5] Enumerate and wrap all mutating controls (delete, bulk delete, scale, restart, apply YAML, Helm mutate, RoleBinding edit/delete, node shell, port-forward, team publish)
- [ ] T029 [US5] Test that fails if a mutating control is not wrapped by `<Gated>` (SC-003)
- [ ] T030 [US5] E2E: alice vs bob differ; API-server-stopped shows "could not verify"; API deny after UI allow shows verbatim error
- [ ] T031 [US5] Invalidate cache after successful RBAC apply and on identity/context change; stale-event discard test

## Phase 6: US3 Teams and users (P2)

- [ ] T032 [US3] Tests first with mocked Rauthy admin API (paging, 401/403, unreachable, stale cache)
- [ ] T033 [US3] Implement `src/backend/idp_directory.rs` (read-only) and `IdpSetAdminKey`, `IdpRefreshDirectory`
- [ ] T034 [P] [US3] UI Teams view: own groups for all, users/groups list for admins (virtualized, priority columns) in `ui/src/features/teams/`
- [ ] T035 [US3] `TeamLink` additive field in `src/backend/team_store.rs` + `LinkTeamProfile/UnlinkTeamProfile`; secret-scan and old-reader compatibility tests
- [ ] T036 [US3] UI filter of shared profiles by group membership, labelled advisory
- [ ] T037 [US3] Fathom-level role map (viewers/editors/admins) gating team publish and settings sync; tests
- [ ] T038 [US3] E2E: alice/bob/carol Story 3 independent test; IdP-unavailable banner

## Phase 7: US4 Role assignment UI (P2)

- [ ] T039 [US4] Tests first: subject rendering with prefixes, escaping of `:` and `/`, missing-prefix warning
- [ ] T040 [US4] Implement `PreviewRoleAssignment` (renders YAML only)
- [ ] T041 [P] [US4] UI `RoleAssignmentDialog` feeding existing `YamlDiffView` + `ConfirmDialog` + `ApplyResourceYaml`; picker from directory
- [ ] T042 [US4] Orphaned-subject detection joined against directory + `OrphanedSubjectBadge` (read-only)
- [ ] T043 [US4] E2E: assign, conflict, denied-by-RBAC disabled state

## Phase 8: US6 LDAP federation (P3)

- [x] T044 [US6] `k8s/rauthy-ldap-lab/` kustomize: OpenLDAP with seed users/groups + Dex LDAP connector, ingress, no object storage
- [x] T045 [US6] Document and script Rauthy upstream-provider registration and group mapping; reconciler job manifest is the default path unless a live probe shows Rauthy upstream mapping/SCIM covers it (no Fathom code)
- [x] T046 [US6] CI guard: `cargo tree` must contain no LDAP crate; live validation per quickstart section 5

## Phase 9: Polish

- [ ] T047 [P] Update `docs/MVP.md` section 14 (Tier B, Tier C, Tier F status) and `specs/014` Story 3 gating note
- [ ] T048 [P] Regression tests for kubeconfig-only path and pre-existing `~/.fathom/` files (SC-006)
- [ ] T049 [P] Performance checks: refresh/capability off UI thread, idle CPU, 200-namespace lazy resolution (SC-004)
- [ ] T050 Run full quickstart on Ubuntu 26.04, Windows 11, macOS 26 (record manual runs where no runner)
- [ ] T051 Run CI gates: fmt, clippy -D warnings, cargo test, lint, typecheck, Vitest, Playwright, binding drift
- [ ] T052 Brochure/marketing PDF screenshots of new screens (always last)

## Dependencies

- Phase 1 -> 2 -> 3. Phase 4 needs Phase 3. Phase 5 needs Phase 4 (identity per context) but its resolver core (T024-T025) can start after Phase 2.
- Phase 6 and 7 need Phase 5. Phase 8 needs only T001/T004 and can run parallel to 5-7.
- Parallel fan-out: T002/T003, T007/T008/T009, T016, T021, T027, T034, T041, and Phase 8 as a whole.

## MVP scope

Phases 1-4 (identity + cluster login), then Phase 5 for the scoped UI value.

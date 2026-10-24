# Tasks: Depths network trust and sessions (052)

**Input**: spec.md, plan.md, research.md, data-model.md, contracts/
**Tests**: required. Targeted tests only; main session runs `scripts/ci.sh` once.

## Phase 1: Setup

- [x] T001 Add `maxminddb` to apps/depths/server/Cargo.toml after confirming deny.toml allows its licence
- [ ] T002 Settings in apps/depths/server/src/config.rs: `DEPTHS_NETWORK_GATE` (true), `DEPTHS_NETWORK_MATURITY_DAYS` (7), `DEPTHS_NETWORK_V4_PREFIX` (32), `DEPTHS_NETWORK_V6_PREFIX` (64), `DEPTHS_TRUSTED_NETWORKS`, `DEPTHS_GEOIP_DB`; invalid values refuse to start

## Phase 2: Foundational

- [ ] T003 `NetworkKey` grouping (IPv4-mapped normalised), `KnownNetwork`, `trust()` returning `TrustDecision` with injected `now`, table tests, in crates/depths-core/src/network.rs
- [ ] T004 `restricted_ok(kind)` table (view/subscribe, Start, Stop, Restart, AnswerNetwork, EndSession) with exhaustive test in crates/depths-core/src/role.rs
- [ ] T005 Persisted registry `networks.json` (users.rs atomic write; ≤64 per person LRU; 180-day prune; `days` max 8; label ≤64 chars) in apps/depths/server/src/networks.rs
- [ ] T006 Session side table (address, browser ≤80 chars, last_recorded; removed on `Ended`) in apps/depths/server/src/session_meta.rs
- [ ] T007 Protocol types: `TrustDecision`, `SessionView`, `NetworkView`, refusal `untrusted_network` in crates/depths-protocol/src/types.rs; regenerate bindings

## Phase 3: US1 Gate (P1)

- [ ] T008 [US1] Record at sign-in (first network atomically, audited) and throttled per request in apps/depths/server/src/{auth,networks}.rs
- [ ] T009 [US1] `gate()` after `permits` in WS dispatch and every mutating HTTP route; audit refusals; close a session's shells on an untrusted request (FR-012) in apps/depths/server/src/{ws,routes,streams}.rs
- [ ] T010 [US1] `/api/me` trust state and `needs_answer`; `POST /api/networks/answer` (never grants trust) in apps/depths/server/src/routes.rs
- [ ] T011 [US1] Server tests: every FR-010 command refused, restricted set allowed, maturity, always-trusted, crafted direct requests; sweep rows in apps/depths/server/tests/
- [ ] T012 [US1] Home/public dialog, restricted banner, controls disabled with reason in apps/depths/web/src/features/network/
- [ ] T013 [US1] Fake backend trust state + Playwright in apps/depths/web/tests/e2e/us5-access/network-gate.spec.ts

## Phase 4: US2 Sessions (P1)

- [ ] T014 [US2] `GET /api/sessions`, `DELETE /api/sessions/{id_hint}`, `POST /api/sessions/end-others`, `GET /api/networks` in apps/depths/server/src/routes.rs (any network, CSRF, audited)
- [ ] T015 [US2] Test that an ended session's sockets close within 5 s and its next request is 401 in apps/depths/server/tests/
- [ ] T016 [US2] Your sessions page (lazy) linked from the account menu, plain-text browser strings, known networks with answer change in apps/depths/web/src/features/sessions/
- [ ] T017 [US2] vitest for formatting helpers; Playwright end-others in apps/depths/web/tests/e2e/us5-access/sessions.spec.ts

## Phase 5: US3 Admin whitelist (P2)

- [ ] T018 [US3] Admin endpoints for a person's sessions/networks; whitelist add/remove and forget (trusted network only; refuse a CIDR containing the requester's own untrusted network) in apps/depths/server/src/routes.rs
- [ ] T019 [US3] Server tests for whitelist from trusted/untrusted, removal reverting to unknown in apps/depths/server/tests/
- [ ] T020 [US3] Users page panel: sessions, networks, whitelist form in apps/depths/web/src/features/users/
- [ ] T021 [US3] Playwright admin whitelist in apps/depths/web/tests/e2e/us5-access/whitelist.spec.ts

## Phase 6: US4 Address correctness (P2)

- [ ] T022 [US4] Proxy-misconfiguration warning (first 20 requests private, no trusted proxies) and admin `proxy_warning` in apps/depths/server/src/client_ip.rs
- [ ] T023 [US4] Optional geo-IP lookup with private-network labels and start-up error logging in apps/depths/server/src/geoip.rs (tests with no file; no network calls)

## Phase 7: Polish

- [ ] T024 docs/guides/depths/security.md (rule, restricted view, settings, proxy, recovery), users.md (sessions, whitelist), install.md (geo-IP download + DB-IP attribution)
- [ ] T025 apps/depths/.env.example entries; spec 050 contracts/config.md cross-reference

## Dependencies

T001–T007 first. US1 before US3 (whitelist needs the gate). US2 independent after T006/T007. Lands after 051 (shares the shell-open path).

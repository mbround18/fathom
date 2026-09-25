# Implementation Plan: Hosted Server Mode (In-Cluster Web App with OIDC Login)

**Branch**: `031-hosted-server-mode` | **Date**: 2026-09-23 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/031-hosted-server-mode/spec.md`

## Summary

Add a third shell over the existing backend: `fathom-server`, a Linux container binary that
serves the built React UI over HTTPS-fronted HTTP and speaks the *same* command/event protocol
over a per-session WebSocket. Sign-in is OIDC authorization-code + PKCE (provider-agnostic; Rauthy
primary, Keycloak second). Each signed-in user gets an isolated `Backend` instance (own thread,
runtime, kube client, watchers, pollers, caps) whose kube client authenticates as that user:
default **pass-through** (the user's OIDC ID token as bearer, which the API server validates via
its own OIDC flags) with an opt-in **impersonation** path (service account with only
`impersonate` rights; `Impersonate-User`/`Impersonate-Group` derived from claims). Tokens are
refreshed by a per-session task and injected per request through a shared token cell, so
long-lived watches, exec and port-forward reconnects always carry a fresh token. The mutation audit
log (spec 026) gains an additive `actor` field. Local-only features (kind provisioning,
kubeconfig/context switching, stdio MCP, fleet) are compiled out of the hosted surface by a
server-side command filter and hidden in the UI via a `mode: "hosted"` capability in
`SubscriptionInfo`. Packaging: distroless static image, Helm chart at `deploy/helm/fathom`, and a
`k8s/` kind overlay with Rauthy + Keycloak and a kind config that wires the API server's OIDC
flags, plus Playwright login e2e against both providers.

## Technical Context

**Language/Version**: Rust edition 2024 (new workspace member `server/`); TypeScript strict + React (transport + small sign-in/no-access UI).

**Primary Dependencies (new)**: `axum` 0.8 (`ws`, `macros`; hyper/tower/tower-http already in the tree via kube), `openidconnect` 4 (over the repo's existing `reqwest`, see research R3), `tower-http` (`ServeDir`, `set-header`, `trace`), `rand`/`getrandom` for session ids, `secrecy` for token wrappers. Already present and reused: `tokio`, `tokio-tungstenite` 0.27 (dedupe verified in T003), `kube` 2 (`Client::new` with custom tower layer), `serde`, `clap`, `tracing`, `ts-rs`.

**Storage**: In-memory session store (spec assumption: re-login after restart is fine). Hosted audit log and per-user state under `FATHOM_STATE_DIR` (default `/var/lib/fathom`, an `emptyDir` or optional PVC via Helm). No secrets persisted.

**Testing**: `cargo test` (session store, CSRF/origin, cookie flags, claim mapping, token cell/refresh with a mock IdP, command filter, per-user limits, audit actor, protocol round-trip for new fields); Vitest for hosted transport + UI gating; Playwright (a) against the scripted fake backend for sign-in/no-access/limit UI states and (b) live login suite `ui/tests/hosted/` against Rauthy and Keycloak on kind (sign-in, renewal, sign-out, expired, revoked, pass-through RBAC for two users). Helm: `helm lint` + `helm template | kubeconform`.

**Target Platform**: Server runs as a Linux container (amd64, arm64). Desktop Tier 1 platforms are unchanged. Browsers: evergreen Chrome/Firefox/Safari.

**Project Type**: Existing multi-crate workspace (`.`, `pipeline`, `ui/src-tauri`, `tools/devbridge`) + one new member `server/`, one new `deploy/` tree, one new `k8s/` overlay.

**Performance Goals**: 25 concurrent users (SC-006) browsing and watching with p95 UI input handling unaffected; idle server CPU stays low (a per-user backend that is idle >= `idleParkMinutes` parks its pollers). Login round trip < 3 s excluding IdP time; no-access state within 3 s of sign-in (SC-008).

**Constraints**: Fail closed (FR-014): no listener bound until OIDC discovery succeeds and config validates. No token/secret ever logged or serialized to the browser (FR-017). Desktop app and devbridge behave as before (FR-018): every backend change is additive behind a `ClusterAccess` seam with the current behavior as the default implementation.

**Scale/Scope**: One home cluster, one replica, tens of users. Memory budget documented per session (target <= 60 MiB/session RSS incl. thread stacks and caches; Helm default limit 2 GiB).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Principle I (Operator-First, Explicit Mutation)**: PASS. Same UI confirmations; hosted adds no implicit writes. Refusals from the API server are surfaced verbatim (forbidden message shown, not generalized). Sign-out and limit-reached are explicit states.
- **Principle II (One Contract, Backend Owns the Cluster)**: PASS. The server is a new *shell* only: it reuses `src/bridge` (pump/hub/`BackendLink`) and the versioned protocol. New protocol additions (`mode`, `identity`, `limits` on `SubscriptionInfo`; `actor` on `AuditLogEntry`) are additive and generated to TS via ts-rs, with round-trip tests. No cluster logic in `server/` beyond client construction.
- **Principle III (Tested at the Boundaries)**: PASS, planned. Every user story has Playwright coverage (fake backend and live-on-kind), unit tests for decision logic (claim mapping, origin/CSRF, filter, limits), round-trip tests for new fields, and binding drift checks.
- **Principle IV (Density)**: PASS. No new layouts beyond sign-in/no-access/limit banners, built with existing shadcn primitives and mobile-first (the web build makes phone use realistic).
- **Principle V (Fast and Quiet)**: PASS with monitoring. Per-user backends must not multiply idle CPU: pollers only run while their view is active (existing behavior), hosted parks idle sessions, and stale events are dropped per session. Risk R-2 tracks thread/runtime cost.
- **Principle VI (User Data Is Sacred)**: PASS. RBAC is the user's, never escalated: pass-through by default; impersonation's service account holds only `impersonate`, and Fathom's own SA never reads on behalf of users. Tokens are wrapped (`secrecy`), never logged. Desktop `~/.fathom` untouched; hosted state is namespaced by `FATHOM_STATE_DIR`.
- **Supported Platforms clause**: **VIOLATION requiring amendment.** The constitution's "Not supported yet" paragraph declares "a browser-hosted web build" out of scope. This feature is exactly that. Resolution: amend to v1.1.0 (MINOR) via `/speckit-constitution` *before implementation* (task T001): add a "Hosted server (Linux container)" tier and reword "Not supported yet" to mobile only. Recorded in Complexity Tracking; `/speckit-analyze` would otherwise flag CRITICAL.
- **Stack clause**: PASS. No component replaced; axum/openidconnect are additions justified in research.md (R2, R3). No pre-release crates (confirm `openidconnect` 4 stable in T003).
- **Workflow gates**: signed commits (verified active), no session links, PR lists user stories.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Constitution "browser-hosted web build out of scope" | The feature's whole purpose | No alternative; amendment is the prescribed path (Governance) |
| Per-user `Backend` instance (thread + runtime each) | Isolation of state, pollers, exec/log sessions is guaranteed by construction; backend code assumes single-user global state (`ExecSessionState` "one active application-wide", `current_context`, `WatchTasks`) | Refactoring ~60 modules to be multi-tenant is far larger and riskier; 25 users makes the cost acceptable (R7) |
| New `server/` crate rather than extending `tools/devbridge` | devbridge is deliberately dev-only, loopback, token-based, and single-subscriber; production code must not live under `tools/` or inherit its fences by accident | Sharing the crate would blur the FR-018 "devbridge unchanged" guarantee |

## Project Structure

### Documentation (this feature)

```text
specs/031-hosted-server-mode/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── hosted-http-api.md      # routes, cookies, CSRF, WS handshake, frame reuse
│   └── hosted-config.md        # env/flags/Helm values, claim mapping, RBAC needs
└── tasks.md
```

### Source Code (repository root)

```text
Cargo.toml                          # + "server" workspace member
server/                             # NEW: fathom-server (lib + bin)
├── Cargo.toml
└── src/
    ├── main.rs                     # clap/env config, fail-closed startup, graceful shutdown
    ├── config.rs                   # HostedConfig + validation (FR-014, FR-015, FR-020)
    ├── oidc.rs                     # discovery, auth-code+PKCE, refresh, end-session, claims
    ├── session.rs                  # in-memory SessionStore, idle/absolute expiry, sweeper
    ├── csrf.rs                     # origin allow-list, double-submit/header CSRF
    ├── http.rs                     # axum router: /auth/*, /api/session, /healthz, /readyz, static UI
    ├── ws.rs                       # /ws upgrade: cookie+origin auth, DevBridge frame reuse per session
    ├── tenant.rs                   # UserBackend: per-session BackendLink+Hub, caps, park, teardown
    ├── kubeauth.rs                 # TokenCell, tower auth layer, pass-through/impersonation clients
    ├── filter.rs                   # hosted command allow-list (denies MutationTarget::Local etc.)
    ├── limits.rs                   # per-user/global session, watch, stream ceilings
    └── audit.rs                    # HostedAuditSink (shared file, actor, viewer filter)

src/backend/
├── access.rs                       # NEW: ClusterAccess trait (Kubeconfig default | injected Hosted)
├── kube.rs                         # route client creation through ClusterAccess
├── context_status.rs, fleet.rs     # gated off when access is hosted
├── audit_log.rs                    # AuditSink seam; `record(..., actor)`
├── persistence.rs                  # state root injectable (FATHOM_STATE_DIR / per-user subdir)
src/bridge/mod.rs                   # spawn_backend_with(access, sink, args) (existing fn delegates)
src/config.rs                       # Args += hosted-facing fields (default = desktop behavior)
src/protocol/v1/
├── bridge_types.rs                 # SubscriptionInfo += mode, identity, limits (all optional)
├── audit.rs                        # AuditLogEntry += actor (serde default)
├── mutation.rs                     # helper: is_hosted_allowed(cmd) classification test table

ui/src/
├── protocol/transport.ts           # + 'hosted' kind
├── protocol/hosted-transport.ts    # NEW: same-origin ws, cookie auth, CSRF header, reauth handling
├── components/hosted/              # NEW: SignInGate, SessionExpired, NoAccess, LimitReached, UserMenu
├── store/handlers/system.ts        # apply mode/identity/limits from SubscriptionInfo
├── (settings/nav)                  # hide kind/kubeconfig/MCP surfaces when mode === 'hosted'
ui/tests/e2e/hosted-*.spec.ts       # fake-backend specs (sign-in gate, no-access, limits, hidden features)
ui/tests/hosted/                    # live login suite vs Rauthy and Keycloak (own playwright config)
ui/playwright.hosted.config.ts

deploy/
├── Dockerfile                      # multi-stage: pnpm build UI, cargo build musl, distroless runtime
└── helm/fathom/                    # Chart.yaml, values.yaml, values.schema.json, templates/*, README.md

k8s/
├── kind-oidc.yaml                  # kind config: apiServer OIDC extraArgs + CA mount + 443 mapping
├── cluster.sh                      # + `up-hosted` (CA gen, CoreDNS rewrite, load image, helm install)
├── addons/{ingress-nginx,rauthy,keycloak}/
├── overlays/hosted/                # rauthy + keycloak + users/groups + RBAC fixtures
└── samples/hosted-rbac.yaml        # ClusterRole/Bindings for alice/bob/carol
```

**Structure Decision**: One new workspace crate (`server/`) as a peer shell of `ui/src-tauri` and
`tools/devbridge`, depending on `fathom` (default-features = false) exactly like devbridge does.
Backend changes are limited to three injection seams (`ClusterAccess`, `AuditSink`, state root)
whose defaults reproduce today's behavior byte-for-byte.

## Phase 0/1 Outputs

research.md (decisions R1-R14), data-model.md, contracts/, quickstart.md. Re-check of the
Constitution gate after design: unchanged (amendment still required; no new violations).

## Parallel Implementation Lanes (memory rule: fan out, max 5 agents, one cargo/pnpm build at a time)

| Lane | Scope | Owns files |
|---|---|---|
| A | Server core: config, oidc, session, csrf, http, ws | `server/src/{config,oidc,session,csrf,http,ws,main}.rs` |
| B | Backend seams + per-user auth: access.rs, kubeauth, tenant, limits, filter, audit | `src/backend/**`, `src/bridge/mod.rs`, `src/protocol/v1/{bridge_types,audit}.rs`, `server/src/{tenant,kubeauth,limits,filter,audit}.rs` |
| C | UI: hosted transport + components + gating | `ui/src/**`, `ui/tests/e2e/hosted-*` |
| D | Packaging: Dockerfile, Helm chart, docs | `deploy/**` |
| E | k8s overlay + live e2e | `k8s/**`, `ui/tests/hosted/**`, `ui/playwright.hosted.config.ts` |

Rule: Foundational (Phase 2) merges before lanes fan out; only one agent runs `cargo`/`pnpm`
build at a time (coordinate via the integration owner); lanes A and B share `server/Cargo.toml`
only in Setup.

# Implementation Plan: Guided OIDC setup

**Branch**: `045-guided-oidc-setup` | **Date**: 2026-10-01 | **Spec**: [spec.md](spec.md)

## Summary

Replace the six bare `Input` fields in `IssuerSettings.tsx` with a stepped path that turns an issuer
URL into a working, verified identity configuration — and that shows the operator every artifact it
reads and every object it writes on the way.

- **Probe before save.** `flow::discover` already takes a bare issuer URL, so step 1 needs no saved
  issuer. What it does not do is keep `scopes_supported`/`claims_supported`, which FR-001 renders, so
  `Discovery` gains two fields and becomes a protocol type.
- **One sign-in, not two.** The draft's tokens live in memory and are committed to the credential
  store at save time (FR-028). This needs a command carrying the draft inline, because
  `OidcSignIn { issuer_id, method }` resolves the issuer out of *saved* config, and `install()`
  writes the refresh token to the keyring before anything else — which a draft must not do (FR-007).
- **Two role steps, never conflated.** `FathomRoleMap`'s three slots gate Fathom features; 042's
  `RoleAssignmentDraft` is real RBAC. Separate steps, separately refusable (FR-027).
- **Read the control plane, write only what we own.** The API server comparison is an ordinary
  namespaced read, offered everywhere including hosted (FR-023). The write is offered only for a
  cluster Fathom provisioned — which needs provenance that does not exist yet (FR-036).
- **Hosted is seeded, not blank.** The server's validated startup config is projected into the
  issuer list, read-only and origin-labelled (FR-031–FR-033), and the configuration surface is
  guarded per user by a `SelfSubjectAccessReview` (FR-034–FR-035).

This feature changes the protocol, the backend, the hosted server and the UI. It is additive on the
wire: new command and event variants only, so the protocol stays v1 (the 034 precedent).

## Technical Context

**Language/Version**: Rust 1.x (backend, hosted server), TypeScript 5 / React 19 (UI)
**Primary Dependencies**: existing only — `kube`, `reqwest`, `keyring`, ts-rs; shadcn/Radix, Zustand
**Storage**: `persistence::load_oidc_config()`/`save_oidc_config()` (desktop); `HostedConfig::from_env` (hosted); keyring via `oidc/store.rs`
**Testing**: `cargo test --all` (incl. `tests/protocol_serde.rs` exhaustive matches), Vitest, Playwright
**Target Platform**: Linux desktop (Tauri) + hosted server
**Project Type**: single workspace, Rust backend + React UI + hosted server crate
**Performance Goals**: discovery and test sign-in are operator-initiated; the API server read is cached per context
**Constraints**: nothing persisted before the explicit save; hosted must not expose kind provisioning; bindings drift is a CI gate
**Scale/Scope**: 6 user stories, 36 FRs, one new protocol module's worth of types

### Files in play

- `src/protocol/v1/oidc.rs` — `Discovery` gains `scopes_supported`/`claims_supported` and the
  `Serialize`/`TS` derives it lacks; new draft, API-server-comparison and seed-origin payloads.
- `src/protocol/v1/cmd.rs`, `event.rs` — new variants; `mutation.rs` — **four** wildcard-free matches
  (`mutation_target` `:46`, `section` `:413`, `audit_summary` `:723`, `hosted_policy` `:1348`) that
  refuse to compile until each new command is classified.
- `src/backend/oidc/flow.rs` — the private `Doc` at `:133` drops the two fields FR-001 needs.
- `src/backend/oidc/session.rs` — a draft sign-in path that does **not** call `store.set` (`:413`)
  and does not collide with the in-flight cancel keyed on `cfg.id` (`:149`).
- `src/backend/cmd/oidc.rs` — new arms beside `OidcSignIn` `:58`; `find_issuer` `:91` stays for the
  saved path, the draft path skips it.
- `src/backend/apiserver_auth.rs` *(new)* — read the `kube-apiserver` static pod, parse the auth
  flags, report a stated unknown on failure (the `detect_kubeadm` 3-state precedent,
  `rbac_access.rs:506`).
- `src/backend/kind.rs` + `src/backend/persistence.rs` — the provenance record of FR-036;
  `summaries_from_names` `:225` hardcodes `profile: None` today.
- `server/src/oidc_seed.rs` *(new)* — project `HostedConfig.oidc`/`claims` into an
  `OidcIssuerConfig` marked seeded; `server/src/filter.rs` `DENIED` — the API server write must be
  listed, because `table()` defaults a new variant to **allowed** (`:170`).
- `ui/src/features/identity/GuidedIssuerPath.tsx` *(new)*, replacing the six inputs in
  `IssuerSettings.tsx`; `IdentityChip.tsx:49` mounts `IssuerSettings` *inside* an open
  `DialogContent`, so the path is a right-side `Sheet`, matching 042's `RoleAssignmentLauncher:47`.
- `ui/src/features/roles/RoleAssignmentLauncher.tsx:19` — `issuers[0]?.id` → the issuer being
  configured (FR-030).
- `ui/src/store/handlers/identity.ts`, `ingest.ts`, plus `store/index.ts:279` **and**
  `handlers/read.ts:32` if a new `ResourceReadEvent` needs custom keying (the two `GenericReadEvent`
  declarations must stay identical).

## Constitution Check

*GATE: passed before Phase 0; re-checked after Phase 1 design (below).*

- **I. Operator-first, explicit mutation.** Every step renders the real artifact — the discovery
  document, the decoded claims, the API server's actual flags, the manifest before it is applied —
  and nothing is persisted before the save (FR-007, FR-022). The wizard tension is real and recorded
  in Complexity Tracking below rather than waved past. ✔ (justified)
- **II. One contract, backend owns the cluster.** All new types are defined once in Rust and
  exported by `cargo test -p fathom --test export_bindings`; the UI reads no cluster itself. The API
  server comparison and write are backend commands, classified in `mutation.rs`'s exhaustive matches
  so a missing decision is a compile error. Additive variants only, so protocol v1 holds (034). ✔
- **III. Tested at the boundaries (NON-NEGOTIABLE).** Each new command and event gets a sample in
  `tests/protocol_serde.rs`'s `all_cmds()`/`all_*_events()` plus its arm in the matching
  `*_is_exhaustive`; bindings drift is checked by `scripts/ci.sh` `job_test`. The draft-writes-nothing
  guarantee (FR-007) and the fail-closed guard (FR-035) are unit-tested at the backend boundary, and
  the step sequence E2E against the fake transport. ✔
- **IV. Density scales with width.** The path is a right-side `Sheet` at 042's width
  (`sm:max-w-xl`), so it layers over the identity chip's own dialog instead of nesting one dialog in
  another, and it inherits the cascade rather than fighting it. ✔
- **V. Fast and quiet.** Discovery and the test sign-in run only when the operator asks. The API
  server read reuses the capability cache's 60s TTL per context. No polling is added. ✔
- **VI. User data is sacred.** The draft's tokens never reach the keyring until save, and are
  discarded on abandon (FR-007). No secret value is rendered or exported (FR-033). The persisted
  issuer format gains only optional fields, and the provenance record of FR-036 is additive. ✔
- **Hosted (constitution platform section).** Hosted gets steps 1–7 including the read-only API
  server comparison, and is denied the kind write — which must be added to `server/src/filter.rs`'s
  `DENIED` list explicitly, since the table defaults new variants to allowed. The seeded issuer is
  read-only there, so hosted gains no config write the constitution withholds. ✔

## Complexity Tracking

| Tension | Why needed | Simpler alternative rejected because |
| --- | --- | --- |
| Principle I says "Wizards, onboarding flows, and abstractions that hide Kubernetes objects are out of scope", and this feature is a stepped guided path. | An OIDC configuration is only correct as a *set*: the issuer URL, the claims the provider actually emits, the prefixes, and the API server's matching configuration. Getting one field wrong produces a sign-in that fails with an opaque message, and today's flat form offers no way to find out which field it was. The path exists to order work the operator must do anyway, and it is resolved the way 043 resolved the setup screen: every step shows the real artifact, every write is confirmed by name, and the path can be abandoned at any step having changed nothing. It sequences objects; it does not hide them. | Keeping the flat form and documenting the ordering was rejected because the failure it produces is undiagnosable from the UI — the operator sees "sign-in failed" and has no view of the discovery document or the decoded claims to compare against. A one-shot "validate" button was rejected because it would have to report every problem at once, in a form that cannot show the artifact each problem is about. |
| FR-036 adds persisted provenance to 030, which is outside this feature's stated surface. | FR-019 and FR-022 both turn on "a cluster Fathom provisioned", and nothing in the codebase can answer that: `kind.rs` derives its list from `kind get clusters` and hardcodes `profile: None` (`:225`), and `persistence.rs` stores nothing about kind at all. Without the record, the write step either cannot be offered or is offered on a false test. | Treating every `kind-*` context as Fathom-owned was rejected because it would rewrite the control plane of a cluster the operator built by hand — precisely what FR-022 exists to prevent. Dropping the write step entirely was rejected because the clarification session chose it deliberately as the third ring of the path. |

## Project Structure

### Documentation (this feature)

```
specs/045-guided-oidc-setup/
├── spec.md
├── plan.md              # this file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/
│   └── protocol.md      # Phase 1
└── checklists/
    └── requirements.md
```

### Source Code (repository root)

```
src/protocol/v1/
├── oidc.rs              # Discovery (+2 fields, +Serialize/TS), draft, comparison, seed payloads
├── cmd.rs               # new BackendCmdV1 variants
├── event.rs             # new SystemEvent / ResourceReadEvent variants
└── mutation.rs          # four exhaustive matches: target, section, audit, hosted policy
src/backend/
├── oidc/flow.rs         # Doc keeps scopes_supported / claims_supported
├── oidc/session.rs      # draft sign-in: no keyring write, no id collision
├── cmd/oidc.rs          # new arms
├── apiserver_auth.rs    # (new) static-pod read, 3-state result
├── kind.rs              # (new) provenance on provision
└── persistence.rs       # (new) provenance store
server/src/
├── oidc_seed.rs         # (new) HostedConfig -> seeded OidcIssuerConfig
└── filter.rs            # DENIED += the API server write
ui/src/features/identity/
├── GuidedIssuerPath.tsx # (new) the Sheet and its steps
├── guided-state.ts      # (new) pure step machine, unit-tested
└── IssuerSettings.tsx   # six inputs removed; list, SignInPanel, RoleMapEditor stay
```

## Phase 1 re-check

The design adds no cluster write beyond the one the spec already confirms, and keeps the new
write behind both the provenance test (FR-036) and hosted's `DENIED` entry. The draft path's only
new persistence is at the explicit save, so Principle VI holds by construction rather than by
convention. Choosing a `Sheet` over a `Dialog` resolved the one structural problem research found in
the UI — `IssuerSettings` is already mounted inside an open dialog — without widening the surface.
All seven bullets above still pass; no gate changed between Phase 0 and Phase 1.

# Implementation Plan: Rauthy OIDC Identity, Teams & Users

**Branch**: `033-rauthy-oidc-teams-users` | **Date**: 2026-09-24 | **Spec**: [spec.md](./spec.md)

## Summary

Add an optional OIDC identity (Rauthy as IdP) to the Rust backend: PKCE login, refresh, credential-store persistence, an OIDC-authenticated kube client per context, a read-only Rauthy directory client, and a capability resolver that generalises 011's `rbac_access.rs`. The UI gains a sign-in surface, a Teams/Users view, a role-assignment picker feeding the existing `ApplyResourceYaml` flow, and a capability gate on every mutating control. LDAP is delivered as IdP-side federation (Dex bridge in a kind lab), with no Fathom LDAP code.

## Technical Context

**Language**: Rust edition 2024 (tokio, kube-rs 2.x, k8s-openapi v1_34); TypeScript strict, React, Vite, shadcn/ui, Vitest, Playwright.
**New dependencies** (stable releases only): `openidconnect`, `keyring`, `arc-swap`, `url`. No LDAP crate.
**Storage**: `~/.fathom/config.json` additive `oidc` key; OS keyring for secrets; 014 team store gains optional `TeamLink` list.
**Testing**: cargo unit + protocol round-trip tests; a mock OIDC provider (in-process, `wiremock`-style) for backend tests; Playwright against the scripted fake backend (Constitution III); live quickstart against kind Rauthy.
**Target platforms**: Tier 1 (Ubuntu 26.04, Windows 11, macOS 26). Keyring backends per OS; loopback listener works on all three.
**Performance**: sign-in/refresh off the UI path; capability lookups cached, lazy per namespace; no idle polling.
**Constraints**: kubeconfig-only path unchanged; tokens never in logs/protocol/`~/.fathom/`.
**Scale**: 200-namespace clusters, directories up to a few thousand users (paged, virtualized list).

## Constitution Check (pre-research and post-design)

| Principle | Assessment |
|---|---|
| I Explicit mutation | Role assignment reuses diff preview + confirm + conflict path; no optimistic UI. PASS |
| II One contract | New commands/events in `src/protocol/v1/`, ts-rs generated, drift-checked. Tokens never cross the protocol. PASS |
| III Tested at boundaries | Round-trip tests per variant, E2E per story with fake backend, ungated-control enumeration test. PASS |
| IV Density | Teams/Roles views mobile-first, priority columns; identity chip collapses to avatar at narrow width. PASS |
| V Fast and quiet | Timer-based refresh, cached capabilities, virtualized directory list, no idle poll. PASS |
| VI User data sacred | Additive config, keyring for secrets, fail-closed capability, never escalates (uses the user's own identity). PASS |
| Stack | New crates justified in research R2; no stack replacement. PASS |

No violations; Complexity Tracking empty. Re-check after Phase 1 design: unchanged.

## Project Structure

### Documentation
`specs/033-rauthy-oidc-teams-users/`: spec.md, research.md, data-model.md, contracts/protocol.md, quickstart.md, plan.md, tasks.md.

### Source (planned)
```text
src/backend/oidc/            # mod.rs, flow.rs (PKCE+device), session.rs (refresh task), store.rs (keyring), validate.rs
src/backend/idp_directory.rs # Rauthy admin API client (read-only)
src/backend/capabilities.rs  # resolver (extends rbac_access.rs), cache
src/backend/kube.rs          # OIDC bearer layer per context (edit)
src/backend/mcp/rbac_guard.rs# use capability resolver (edit)
src/backend/team_store.rs    # TeamLink (additive) (edit)
src/protocol/v1/oidc.rs, idp.rs, capabilities.rs
ui/src/features/identity/    # SignInPanel, IdentityChip, IssuerSettings
ui/src/features/teams/       # TeamsView, UserList, GroupList
ui/src/features/roles/       # RoleAssignmentDialog, OrphanedSubjectBadge
ui/src/lib/capabilities.ts   # useCapability hook, <Gated> wrapper
k8s/rauthy-clients/          # kustomize: fathom-desktop client + read-only API key
k8s/rauthy-ldap-lab/         # kustomize: OpenLDAP + Dex
k8s/kind-oidc-apiserver/     # kubeadm patch + notes
```

**Structure decision**: extend the existing single Rust crate and `ui/` app; one new `oidc` module and two small backend modules.

## Phasing

1. **Phase 0 lab and research validation**: verify R1/R3/R4 risks against kind Rauthy, decide fallbacks.
2. **Phase 1 identity** (US1): protocol, flow, keyring, session, UI sign-in.
3. **Phase 2 cluster login** (US2): kube bearer layer, generator prefill.
4. **Phase 3 capabilities and scoped UI** (US5): resolver, gate, MCP reuse. Delivered before US3/US4 because they depend on it.
5. **Phase 4 directory, teams, role assignment** (US3, US4).
6. **Phase 5 LDAP federation lab** (US6).
7. **Phase 6 polish**: docs, MVP.md updates, brochure screenshots last.

## Complexity Tracking

None.

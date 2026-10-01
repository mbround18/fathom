# Phase 0 research: guided OIDC setup

Everything below was read in the tree at `f5b75ae`. Two findings shrank the work; four found
problems the spec or the plan had to absorb, and one of those disproved an assumption in the spec.

## Starting point in the code

- `src/backend/oidc/flow.rs` — `discover(http, issuer_url: &str)` `:132`, `Discovery` `:48`, private
  `Doc` `:133`. Browser and device-code flows, PKCE, JWKS validation and claim decoding all exist.
- `src/backend/oidc/session.rs` — `sign_in(cfg: OidcIssuerConfig, method)` `:148`, `install()` `:397`.
- `src/backend/oidc/store.rs` — keyring naming `:46`, sticky degraded-memory flag `:149`.
- `src/backend/cmd/oidc.rs` — 7 arms; `find_issuer` `:91` is the only id→config lookup.
- `src/protocol/v1/oidc.rs` — `OidcIssuerConfig` `:29`, `FathomRoleMap` `:136`.
- `src/backend/capabilities.rs:272` / `rbac_access.rs:29` — the verbatim-denial primitive.
- `ui/src/features/identity/IssuerSettings.tsx` — 164 lines, six inputs, no props.
- `ui/src/features/roles/RoleAssignmentLauncher.tsx` / `RoleAssignmentDialog.tsx` — 042's precedent.

## R1 — Inline-draft sign-in is a one-arm change, not a refactor

**Decision**: add one command that carries the whole `OidcIssuerConfig` inline, and route it to the
existing `sign_in`. **Rationale**: `find_issuer` (`cmd/oidc.rs:91`) is the *only* place an id is
turned into a config, and `sign_in` already takes the config **by value** (`session.rs:148`), so
everything downstream is draft-ready as written. **Alternatives**: saving the issuer first and
deleting it on abandon was rejected — it violates FR-007 and leaves wreckage if the process dies.

## R2 — But a draft sign-in must not take the keyring path, and must not reuse the id

**Decision**: the draft flow publishes to the in-memory `Live` slot (`session.rs:455`) and skips the
`store.set` calls; the commit at save writes the refresh token under the now-saved id. The draft is
tracked under a key that cannot collide with a saved issuer. **Rationale**: `install()` writes the
refresh token *first* (`:413`), before anything else — a naive draft sign-in would persist a
credential for an issuer that was never saved, breaking FR-007 outright. Separately, `sign_in`
cancels any in-flight sign-in for `cfg.id` (`:149`), so a draft that reuses a saved id would kill
the operator's live session mid-edit.

## R3 — `Discovery` is not a protocol type, and drops two fields FR-001 renders

**Decision**: add `scopes_supported` and `claims_supported` to the private `Doc` (`flow.rs:133`,
6 fields today) and give `Discovery` `Serialize` + `TS` beside its existing `Debug, Clone, PartialEq,
Eq`. **Rationale**: FR-001 requires showing what the provider advertises; those are the two fields
that say it, and they are parsed away today. **Alternatives**: a separate UI-only summary type was
rejected — Principle II wants the type defined once.

## R4 — The path is a `Sheet`, not a `Dialog`

**Decision**: `Sheet side="right" className="w-[calc(100vw-2rem)] sm:max-w-xl"`, body guarded by
`{open && …}`, exactly as `RoleAssignmentLauncher.tsx:47`. **Rationale**: FR-029 says "a dialog, like
042" — but 042 is *already* a Sheet, and the reason matters here: `IssuerSettings` is mounted
**inside an open `DialogContent`** at `IdentityChip.tsx:49` (`sm:max-w-2xl max-h-[85vh]`), so an
unconditional `Dialog` would nest one dialog in another. A Sheet layers over it. The step sequence
itself goes in a pure `guided-state.ts`, following `components/setup/setup-state.ts` ("Pure, so every
transition is unit tested"), rather than 042's branch-on-state (`RoleAssignmentDialog.tsx:137`),
which does not scale to eight steps.

## R5 — The ownership test is not answerable today (disproved a spec assumption)

**Decision**: persist provenance at provision time; treat an unrecorded kind cluster as not owned.
Recorded as **FR-036** and in the plan's Complexity Tracking. **Rationale**: `summaries_from_names`
hardcodes `profile: None` (`kind.rs:225`), the cluster list is derived from `kind get clusters`, and
`persistence.rs` contains **zero** kind references — there is no provenance of any kind. The spec had
assumed 030 could answer "did Fathom create this"; it cannot. **Alternatives**: treating every
`kind-*` context as owned was rejected because it would rewrite a hand-built control plane, which is
what FR-022 exists to prevent.

## R6 — The lab script is a reference, not something to port

**Decision**: implement the API server write in the backend with a backup, a readiness wait and a
verbatim error, and keep the write inline in the handler. **Rationale**: `k8s/kind-oidc-apiserver/
apply.sh` is 18 lines marked "Lab only" — `docker exec` by container name, `cp -n … || true`, one
brittle `sed` anchored on `- --authorization-mode=Node,RBAC`, **no readiness wait, no rollback, no
podman path**. Separately, `kind.rs`'s `Runner` trait (`:47`) is sync, stdin-less and probe-only, and
create/delete already bypass it, so there is no existing exec-into-node abstraction to reuse.
Inline matters for FR-024: `audit_summary()` is taken at dispatch (`src/backend.rs:312`), so a write
that completes asynchronously would be audited before it happened.

## R7 — Hosted already has the config; what it lacks is the bridge

**Decision**: project `HostedConfig.oidc`/`claims` into an `OidcIssuerConfig` flagged as seeded.
**Rationale**: `HostedConfig::from_env` (`server/src/config.rs:158`) already requires
`FATHOM_OIDC_ISSUER`/`FATHOM_OIDC_CLIENT_ID`, reads scopes, claims and prefixes, and supports
`FATHOM_OIDC_CLIENT_SECRET_FILE` for a mounted file — the mounted-Secret pattern already. Grepping
`server/src/` for `OidcIssuerConfig`, `load_oidc_config` or `save_oidc_config` returns **nothing**:
the two representations are structurally near-identical and never connected, which is why the
identity UI would render an empty issuer list in a server authenticating against one.

## R8 — Protocol mechanics and gates

**Decision**: additive variants, protocol stays **v1**. New commands must be classified in four
wildcard-free matches in `mutation.rs` (`:46`, `:413`, `:723`, `:1348`) — a missing decision is a
compile error — and the API server write must be added to `server/src/filter.rs`'s `DENIED` (`:93`),
because `table()` defaults an unknown variant to **allowed** (`:170`), so an omission silently
permits it in hosted. Bindings come from `cargo test -p fathom --test export_bindings`; drift is
caught by `scripts/ci.sh` `job_test`. **Rationale**: the constitution requires a version bump only
for changes that break existing clients (`constitution.md:41`); 034 added commands and events and
stayed v1, which is the recorded precedent.

## Open questions

None blocking. Two carried into tasks: whether the provenance record of FR-036 belongs beside the
UI config or in its own file under the state directory (a data-model choice, not a behavioural one),
and whether the API server write reaches the node by extending `Runner` with an async stdin-capable
exec or by a short-lived privileged pod.

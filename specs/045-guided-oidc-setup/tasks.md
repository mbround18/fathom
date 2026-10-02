# Tasks: Guided OIDC setup

**Input**: Design documents from `/specs/045-guided-oidc-setup/`

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md), [data-model.md](data-model.md), [contracts/protocol.md](contracts/protocol.md)

**Tests**: Included, and not optional here — Constitution Principle III is NON-NEGOTIABLE: "Every protocol command and event variant MUST have a serialization round-trip test", and the boundaries this feature adds (a draft that must persist nothing, a read that must state its unknowns, a write gated on provenance) are exactly the kind the constitution requires covered. Test tasks precede the implementation they cover within each phase.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US6)
- Exact file paths are given in every task

---

## Phase 1: Setup

**Purpose**: Establish the baseline the rest of the work is measured against. No project initialization — this is an existing workspace.

- [X] T001 Record a green baseline by running `scripts/ci.sh all` and noting any pre-existing failure in this file, so later gate failures are attributable to 045 (`CARGO_BUILD_JOBS=4` if other agents are active)
  **Baseline, recorded 2026-10-01 (late — see below).** `scripts/ci.sh all` on
  `171cd9f`: `check` passed in 110s, `test` passed in 216s, `ui` **FAILED** after
  272s. The ui failure was *not* a test failure: vitest reported
  `Test Files  105 passed (105)` and then died on worker startup —
  `[vitest-pool]: Failed to start forks worker for test files .../imageInspectPanel.test.tsx`,
  `Caused by: [vitest-pool-runner]: Timeout waiting for worker to respond`. Load
  average was 46 at the time, with two agent lanes and the dagger engine on one
  32-core / 31 GB box, and the named file has nothing to do with 045. The same
  suite run directly was `Tests  1661 passed (1661)`. So: **a vitest worker-startup
  timeout inside the dagger container under high load is a known pre-existing
  flake, not attributable to 045.** Treat a `ui` stage failure as real only if a
  `test result`/`Tests` line actually reports failures.

  This task was recorded after the fact rather than before, which is the honest
  version: the baseline was supposed to make a late gate failure attributable, and
  instead attribution had to be reconstructed from the log. Doing it first would
  have been cheaper.
- [X] T002 [P] Create the empty module files so later tasks only edit: `src/backend/apiserver_auth.rs`, `server/src/oidc_seed.rs`, `ui/src/features/identity/guided-state.ts`, `ui/src/features/identity/GuidedIssuerPath.tsx`, and register `mod apiserver_auth;` in `src/backend.rs` and `mod oidc_seed;` in `server/src/main.rs` (or `lib.rs`, whichever declares the modules)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The protocol surface and the step machine every user story renders through. Nothing in Phase 3+ compiles without this.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T003 Extend the private `struct Doc` in `src/backend/oidc/flow.rs:133` with `scopes_supported: Vec<String>` and `claims_supported: Vec<String>` (both `#[serde(default)]`, since a provider may omit either), and carry them onto `Discovery` at `flow.rs:48`
- [X] T004 Add `Serialize` and `TS` to `Discovery`'s derives in `src/backend/oidc/flow.rs:48` (it has `Debug, Clone, PartialEq, Eq` today) and re-export it from `src/protocol/v1/oidc.rs` so the UI can receive it
- [X] T005 [P] Add `IssuerDraft { config: OidcIssuerConfig, method: OidcSignInMethod }` and `DraftSignInResult { username: String, groups: Vec<String>, claims: Vec<ClaimPair>, expires_at: i64 }` to `src/protocol/v1/oidc.rs` with `#[derive(Debug, Clone, Serialize, Deserialize, TS)]`; `expires_at` needs `#[ts(type = "number")]`. `DraftSignInResult` MUST NOT carry token material of any kind
- [X] T006 [P] Add `ApiServerAuthStyle { Legacy, Structured, None, Unknown }`, `ApiServerAuthRead`, `AuthDifference { field: String, cluster: Option<String>, draft: Option<String> }` and `ApiServerAuthComparison` to `src/protocol/v1/oidc.rs` with the exact fields in [data-model.md](data-model.md); `Option<String>` means *absent*, never empty-string, and `ApiServerAuthRead` carries both `checked: bool` and `reason: Option<String>`
- [X] T007 [P] Add `IssuerOrigin { Saved, SeededFromEnvironment }` to `src/protocol/v1/oidc.rs`
- [X] T008 Add the five command variants from [contracts/protocol.md](contracts/protocol.md) to `BackendCmdV1` in `src/protocol/v1/cmd.rs` — `OidcDiscoverDraft`, `OidcTestSignIn`, `OidcSaveDraft`, `ReadApiServerAuth`, `WriteApiServerAuth` — each with a `// 045-guided-oidc-setup:` comment naming its reply event, and add the new payload types to the `use super::{…}` block at `cmd.rs:4`
- [X] T009 Add `OidcDraftDiscovered`, `OidcDraftSignInSucceeded` and `OidcSeedOrigin` to `SystemEvent` (`src/protocol/v1/event.rs:146`) and `ApiServerAuthRead` to `ResourceReadEvent` (`event.rs:724`), extending the import blocks at `event.rs:2`
- [X] T010 Classify all five new commands in the four wildcard-free matches in `src/protocol/v1/mutation.rs` — `mutation_target()` `:46`, `section()` `:413`, `audit_summary()` `:723`, `hosted_policy()` `:1348` — per [contracts/protocol.md](contracts/protocol.md): only `ReadApiServerAuth` is `HostedPolicy::Allowed`; `OidcDiscoverDraft` is `Denied(ServerSideProbe)`, `OidcTestSignIn` and `OidcSaveDraft` are `Denied(SharedState)` (the group every other OIDC config command is already in), `WriteApiServerAuth` is `Denied(HostedDenial::Kind)`. `WriteApiServerAuth` is the only audited one, under a new `AuditActionKind::ApiServerAuthChange`
- [X] T011 Add `"WriteApiServerAuth"`, `"OidcDiscoverDraft"`, `"OidcTestSignIn"` and `"OidcSaveDraft"` by name to the `DENIED` const in `server/src/filter.rs:93`. This is not redundant with T010: `table()` derives from `declared_variants()` and defaults an unrecognised variant to **allowed** (`filter.rs:170`), so omitting it silently permits the write in hosted
- [X] T012 Add samples for all five commands to `all_cmds()` in `tests/protocol_serde.rs:1480` and their arms to `all_cmds_is_exhaustive` `:2285`; add the three system-event samples to `all_system_events()` `:2561` and `ApiServerAuthRead` to `all_resource_read_events()` `:3322`, with arms in each `*_is_exhaustive`
- [X] T013 [P] Add per-feature round-trip tests `guided_oidc_cmd_roundtrips()` to `src/protocol/v1/cmd.rs`'s `mod tests` (`:1194`) and `guided_oidc_event_roundtrips()` to `src/protocol/v1/event.rs`'s `mod tests` (`:965`), asserting the `type` tag string as 042 does at `cmd.rs:1398`
- [X] T014 Regenerate bindings with `cargo test -p fathom --test export_bindings` and commit `ui/src/protocol/generated/`; confirm `scripts/ci.sh test`'s drift step passes
- [X] T015 [P] Write unit tests for the pure step machine in `ui/tests/unit/guided-state.test.ts` covering every transition of `GuidedStep`, that `bindings` and `apiserver-write` are unreachable before a save, and that a `SeededFromEnvironment` origin freezes the config and skips `review-save`; include walking every step backwards without losing the draft (FR-025)
- [X] T016 Implement `ui/src/features/identity/guided-state.ts` as a pure reducer over `GuidedDraft` per [data-model.md](data-model.md), following `ui/src/components/setup/setup-state.ts` ("Pure, so every transition is unit tested") rather than 042's branch-on-state; `back`/`goto` keep the draft and the path stays closable at every step (FR-025)
- [X] T017 Implement the shell in `ui/src/features/identity/GuidedIssuerPath.tsx` as `Sheet side="right" className="w-[calc(100vw-2rem)] sm:max-w-xl"` with the body guarded by `{open && …}`, matching `ui/src/features/roles/RoleAssignmentLauncher.tsx:47`. It MUST be a Sheet, not a Dialog: `IssuerSettings` is already mounted inside an open `DialogContent` at `ui/src/features/identity/IdentityChip.tsx:49`, so a Dialog would nest (FR-029)
- [X] T018 Add the new events to the identity store handler `ui/src/store/handlers/identity.ts` and its event union, route them in `ui/src/store/ingest.ts`, and if `ApiServerAuthRead` needs custom keying exclude it from `GenericReadEvent` in **both** `ui/src/store/index.ts:279` and `ui/src/store/handlers/read.ts:32` — the comment at `index.ts:282` requires the two declarations stay identical

**Checkpoint**: `cargo test --all` and `pnpm test` green; the Sheet opens empty with no steps wired.

---

## Phase 3: User Story 1 — Add an identity provider from its URL alone (Priority: P1) 🎯 MVP

**Goal**: An operator pastes an issuer URL and sees what the provider actually advertises, before anything is saved.

**Independent Test**: Open **Add issuer…**, paste the lab Keycloak URL, and confirm the endpoints, supported scopes and supported claims are shown; paste a URL whose discovery document reports a different `issuer` and confirm the path refuses to continue, showing both values.

- [X] T019 [P] [US1] Backend test in `src/backend/oidc/flow.rs`'s test module that `Doc` retains `scopes_supported` and `claims_supported`, and that both default to empty when the provider omits them (FR-001)
- [X] T020 [P] [US1] Backend test that a discovery document whose `issuer` differs from the URL requested is reported as a mismatch carrying **both** values, not a generic failure (FR-002)
- [X] T021 [US1] Handle `OidcDiscoverDraft` in `src/backend/cmd/oidc.rs` beside the `OidcSignIn` arm at `:58`, calling the existing `flow::discover(http, issuer_url)` (`flow.rs:132`, already takes a bare URL — no saved issuer is needed) and emitting one `OidcDraftDiscovered`
- [X] T022 [US1] Return discovery failures as `CommandFinished { ok: false, error }` carrying the verbatim transport or HTTP error and the URL tried, and honour `ca_bundle_path` from the draft so a private CA can be supplied before the first successful probe (FR-003)
- [X] T023 [US1] Implement the issuer-URL and discovery steps in `ui/src/features/identity/GuidedIssuerPath.tsx`, rendering endpoints, `scopes_supported` and `claims_supported` as the real document rather than a summary, with the issuer-mismatch refusal showing both values (FR-001, FR-002)
- [X] T024 [P] [US1] E2E in `ui/tests/e2e/guided-oidc.spec.ts` against the fake transport: paste a URL, assert `OidcDiscoverDraft` was sent, assert the rendered scopes and claims, then assert the mismatch case blocks progress (SC-001, SC-002)

**Checkpoint**: US1 is independently usable — the path probes and reports, and still writes nothing.

---

## Phase 4: User Story 2 — See the claims a real token actually carries (Priority: P1)

**Goal**: A complete sign-in against an unsaved draft, showing the decoded claims and the identity as Kubernetes will see it — with nothing persisted.

**Independent Test**: Run the test sign-in from a draft, confirm the claims and the prefixed username/groups preview appear, then close the Sheet and confirm the issuer list is unchanged and no keyring entry exists.

- [X] T025 [P] [US2] Backend test that a draft sign-in writes **no** keyring entry: assert `store.set` is never called for the draft's id and that `persistence::load_oidc_config()` is byte-identical before and after (FR-007, SC-004)
- [X] T026 [P] [US2] Backend test that a draft sign-in for an id matching a saved, signed-in issuer does **not** cancel that issuer's live session — `sign_in` cancels any in-flight sign-in keyed on `cfg.id` at `src/backend/oidc/session.rs:149`, which a draft must not trigger
- [X] T027 [US2] Add a draft sign-in path in `src/backend/oidc/session.rs` that reaches the existing flow (`sign_in` already takes `OidcIssuerConfig` **by value** at `:148`) but publishes only to the in-memory `Live` slot (`:455`) and skips the `store.set(SecretKind::RefreshToken, …)` calls that `install()` performs first at `:413`; track the draft under a key that cannot collide with a saved issuer id
- [X] T028 [US2] Handle `OidcTestSignIn` in `src/backend/cmd/oidc.rs`, taking the config from the inline draft rather than `find_issuer` (`:91`), reusing the existing browser and device-code progress events and emitting `OidcDraftSignInSucceeded` on success (FR-004)
- [X] T029 [US2] Build `DraftSignInResult` from the validated ID token's claims in the draft path, carrying claim names and values but no token material (FR-005)
- [X] T030 [US2] Implement the claims step in `ui/src/features/identity/GuidedIssuerPath.tsx`: show every claim with its value, let the username and groups claims be chosen from the claims actually present, and preview the username and group list with the configured prefixes applied (FR-005, FR-006)
- [X] T031 [US2] Expose `username_prefix` and `groups_prefix` as editable fields showing their current values in the same step, since they decide whether bindings match (FR-011)
- [X] T032 [P] [US2] E2E in `ui/tests/e2e/guided-oidc.spec.ts`: complete a scripted test sign-in, assert the claims table and the prefixed preview, then close the Sheet and assert no `OidcSaveDraft` was sent and the issuer list is unchanged (SC-004, SC-007)

**Checkpoint**: US1 and US2 both work independently; the draft still persists nothing.

---

## Phase 5: User Story 3 — Reconcile with what the cluster will accept (Priority: P1)

**Goal**: Show, field by field, where Fathom's draft and the API server's own authentication configuration disagree — and say so honestly when it cannot be read.

**Independent Test**: On the dev cluster, reach the comparison step and confirm it names the context and lists a row per difference; then point Fathom at a managed or RBAC-restricted context and confirm it states the unknown verbatim without blocking the path.

- [X] T033 [P] [US3] Backend tests in `src/backend/apiserver_auth.rs` for parsing both styles out of the `kube-apiserver` static pod: legacy `--oidc-issuer-url`/`--oidc-client-id`/`--oidc-username-claim`/`--oidc-groups-claim`/`--oidc-username-prefix`/`--oidc-groups-prefix`, and a structured `--authentication-config` file; assert the two are never reported together, since they are mutually exclusive (FR-008, FR-020)
- [X] T034 [P] [US3] Backend test that a denied or impossible read returns a **successful** command with `checked: false`, `style: Unknown` and the API server's verbatim `reason` — never an error and never an assumed pass, following the three-state `detect_kubeadm` precedent at `src/backend/rbac_access.rs:506` (FR-010)
- [X] T035 [US3] Implement the static-pod read in `src/backend/apiserver_auth.rs`: read the `kube-apiserver` pod in `kube-system` as the signed-in user, extract the flags, and return `ApiServerAuthRead` including the context it read from
- [X] T036 [US3] Implement the field-by-field comparison producing `Vec<AuthDifference>` over issuer URL, client id/audience, username claim, groups claim, username prefix and groups prefix, distinguishing *absent* (`None`) from *different* (FR-009)
- [X] T037 [US3] Handle `ReadApiServerAuth` in `src/backend/cmd/oidc.rs`, emitting one `ApiServerAuthRead` event, and reuse the capability cache's 60s per-context TTL (`src/backend/capabilities.rs:272`) rather than re-reading on every step entry
- [X] T038 [US3] Implement the comparison step in `ui/src/features/identity/GuidedIssuerPath.tsx`: a row per difference, the context named, and the stated-unknown rendering that does not block progress (FR-008–FR-010)
- [X] T039 [US3] Re-read the comparison, or state that it is stale, when the active context changes while the path is open (FR-026)
- [X] T040 [P] [US3] E2E in `ui/tests/e2e/guided-oidc.spec.ts` for three scripted replies: a clean match, a prefix mismatch reported before any save, and an unreadable API server shown verbatim (SC-003)

**Checkpoint**: All three P1 stories work independently. This is the point where the path is more useful than the flat form.

---

## Phase 6: User Story 4 — Map groups to roles, and create the bindings (Priority: P2)

**Goal**: Fill Fathom's three role slots from the groups the sign-in actually returned, and separately hand a group to 042's binding form — without either being mistaken for the other.

**Independent Test**: Fill the three slots from observed groups and confirm the step says they grant nothing in the cluster; then after saving, take the bindings handoff and confirm the subject arrives pre-filled with the API server's prefix applied and carries the issuer just configured.

- [X] T041 [P] [US4] Unit test in `ui/tests/unit/guided-state.test.ts` that the Fathom role slots and the cluster bindings are two separate, separately refusable steps, and that refusing the bindings step leaves a saved issuer intact (FR-027)
- [X] T042 [US4] Implement the Fathom role-slot step in `ui/src/features/identity/GuidedIssuerPath.tsx`, offering the groups observed in the test sign-in plus free text for `viewers_group`, `editors_group` and `admins_group`, and stating that these gate Fathom's own features and grant nothing in the cluster (FR-012, FR-027)
- [X] T043 [US4] Fix `ui/src/features/roles/RoleAssignmentLauncher.tsx:19` to take the issuer being configured as a prop instead of reading `issuers[0]?.id`, which is wrong as soon as a second issuer exists, keeping the `if (!issuerId) return null` guard at `:29` (FR-030)
- [X] T044 [US4] Implement the bindings handoff from the path into the existing role-binding form with the subject pre-filled including the API server's prefix, reached only after the save because 042's `RoleAssignmentDraft` requires a saved `issuer_id` (FR-013)
- [X] T045 [P] [US4] E2E in `ui/tests/e2e/guided-oidc.spec.ts` with two saved issuers, asserting the handoff carries the second one's id and that the two role steps are labelled distinctly (FR-030)

**Checkpoint**: US1–US4 all independently functional.

---

## Phase 7: User Story 5 — Save, and reopen to review (Priority: P2)

**Goal**: One transactional save that also establishes the session, and a path that opens pre-filled for an existing issuer — replacing the flat form rather than sitting beside it.

**Independent Test**: Save after a successful test sign-in and confirm you are signed in without a second sign-in; reopen the issuer with **Review…** and confirm the path starts pre-filled at step 1 with the same probe available.

- [X] T046 [P] [US5] Backend test that `OidcSaveDraft` with `adopt_test_session: true` persists the issuer and the role map **and** commits the test sign-in's refresh token under the now-saved id, leaving the operator signed in with exactly one sign-in (FR-028)
- [X] T047 [P] [US5] Backend test that `OidcSaveDraft` is refused when `adopt_test_session` is set but no test sign-in is live, and that `save_oidc_config`'s existing refusal to overwrite unparseable JSON (`src/backend/persistence.rs:419`) still holds
- [X] T048 [P] [US5] Backend test that saving an issuer whose id is already in use is refused with an explanation rather than silently merging (FR-017)
- [X] T049 [US5] Handle `OidcSaveDraft` in `src/backend/cmd/oidc.rs` as a single transactional save: validate, persist the issuer via `save_oidc_config`, persist the role map, then adopt the draft's in-memory tokens into the credential store under the saved id (FR-028)
- [X] T050 [US5] Implement the final summary step in `ui/src/features/identity/GuidedIssuerPath.tsx` listing every field to be stored and recording which verification steps were performed and which were skipped (FR-014)
- [X] T051 [US5] Offer **Save without testing** at every step, naming what has not been verified, and carry that into the summary (FR-015)
- [X] T052 [US5] Open the same path pre-filled from an existing issuer's **Review…**, and warn before saving a change to an issuer that currently has a live session (FR-016)
- [X] T053 [US5] Remove the six bare `Input` fields from `ui/src/features/identity/IssuerSettings.tsx` and mount the path for both add and edit, keeping the issuer list, `SignInPanel` and `RoleMapEditor` (`:93`); also drop the dead `s.identity.errors['']` read at `:49`, which `reduceIdentity` never writes (FR-018)
- [X] T054 [US5] Mount the path unchanged from all three existing surfaces — `IdentityChip.tsx:49`, the Settings pane via `SignInSettings` (`PaneHost.tsx:483`) and `SetupSignIn.tsx:43` — verifying the Sheet layers over the chip's open dialog instead of nesting (FR-029)
- [X] T055 [P] [US5] E2E in `ui/tests/e2e/guided-oidc.spec.ts`: save-then-signed-in with a single sign-in, reopen pre-filled, duplicate-id refusal, and the save-without-testing summary (SC-005, SC-007)

**Checkpoint**: The path fully replaces the flat form. US1–US5 complete.

---

## Phase 8: User Story 6 — Configure the API server of a cluster Fathom provisioned (Priority: P3)

**Goal**: Close the loop on a cluster Fathom owns — and prove it owns it before writing anything.

**Independent Test**: On a kind cluster Fathom provisioned, apply the configuration and complete a real sign-in against the cluster; on a kind cluster created by hand outside Fathom, confirm the write is **not** offered and copyable text is shown instead.

- [X] T056 [P] [US6] Backend test that a kind cluster with no provenance record is reported as **not owned**, including one created outside Fathom whose context name looks identical — the ownership test must not be derived from the `kind-` context name (FR-036, FR-022)
- [X] T057 [US6] Add the provenance record of FR-036 to `src/backend/persistence.rs` (`cluster`, `provisioned_at`, `profile`), written when a provision succeeds and removed when Fathom deletes the cluster; `persistence.rs` has no kind references today
- [X] T058 [US6] Populate the record from the provision path in `src/backend/kind.rs` and stop hardcoding `profile: None` in `summaries_from_names` (`:225`), reading the profile from the record instead
- [X] T059 [P] [US6] Backend tests in `src/backend/apiserver_auth.rs` for the write: a backup is taken before any change, the control plane is waited for after it, and a failure reports the verbatim error (FR-021)
- [X] T060 [US6] Implement the API server write, in the style the cluster already uses and never both, refusing unless the named cluster carries the provenance record (FR-019, FR-020, FR-022). `k8s/kind-oidc-apiserver/apply.sh` is a reference only — it is 18 lines marked "Lab only" with no readiness wait, no rollback and a `sed` anchored on `- --authorization-mode=Node,RBAC`
- [X] T061 [US6] Handle `WriteApiServerAuth` **inline** in `src/backend/cmd/oidc.rs`: `audit_summary()` is taken at dispatch (`src/backend.rs:312`), so a write that completed asynchronously would be audited before it happened (FR-024)
- [X] T062 [US6] Implement the write step in `ui/src/features/identity/GuidedIssuerPath.tsx`, showing the backup it will take and the exact flags it will set, behind an explicit confirmation that names them (FR-019, SC-007)
- [X] T063 [US6] Show the copyable required-configuration text instead of the write on any context without the provenance record, saying plainly that Fathom will not change a control plane it does not own (FR-022)
- [X] T064 [P] [US6] E2E in `ui/tests/e2e/guided-oidc.spec.ts` asserting the write is offered for a provisioned cluster and replaced by copyable text for an unowned one, and a live check in the restricted suite that the write appears in the mutation audit log (FR-024)

**Checkpoint**: All six user stories independently functional on the desktop build.

---

## Phase 9: Hosted mode

**Purpose**: Hosted gets steps 1–7 including the read, is denied the write, is seeded from its own startup configuration, and gates the whole surface on the cluster's answer. These cut across US1–US5 rather than belonging to one of them.

- [X] T065 [P] Test in `server/src/oidc_seed.rs` that the projection maps `oidc.issuer_raw` verbatim (a `Url` would append a `/`), carries scopes, claims and prefixes with empty prefixes becoming `None`, carries `ca_file` to `ca_bundle_path`, sets `admin_api_base: None`, and **never** projects `client_secret` (FR-033)
- [X] T066 Implement the projection in `server/src/oidc_seed.rs` from the already-validated `HostedConfig` (`server/src/config.rs:157`) into an `OidcIssuerConfig` marked `IssuerOrigin::SeededFromEnvironment`, introducing no second source of truth
- [X] T067 Surface the seeded issuer in the hosted issuer list and emit `OidcSeedOrigin` naming its source, so the UI can render it read-only; desktop never emits it and `Saved` is the absence of it (FR-031)
- [X] T068 Render a seeded issuer read-only in `ui/src/features/identity/GuidedIssuerPath.tsx` and `IssuerSettings.tsx`: not editable, not deletable, origin stated, the place it is changed named — while discovery, claims and the API server comparison all stay available (FR-032)
- [X] T069 Document the mounted Secret/ConfigMap and the `FATHOM_OIDC_*` env overrides in the hosted manifests under `k8s/`, so the seed appears in manifest export like every other object Fathom would apply, with no secret value rendered (FR-033)
- [X] T070 [P] Test that the hosted configuration surface is gated by a `SelfSubjectAccessReview` performed as the signed-in user, reusing `review_resource_access` (`src/backend/rbac_access.rs:29`) and `CapabilityResult`'s verbatim `reason` (`src/backend/capabilities.rs:272`), and that a failed review **fails closed** with the unknown stated verbatim (FR-034, FR-035)
- [X] T071 Implement that guard on the hosted OIDC configuration surface, with no Fathom-internal role list anywhere in the decision path (FR-034, FR-035)
- [X] T072 Show the identity surface in hosted, which `ui/src/components/layout/TopBar.tsx:109` hides today via `{!isHosted && <IdentityChip …>}` — the Settings sign-in pane must reach the path even if the chip stays hidden
- [X] T073 [P] Hosted E2E in `ui/tests/e2e/hosted-guided-oidc.spec.ts` (run by `pnpm e2e:hosted`): the seeded issuer renders read-only with its origin, steps 1–7 including the comparison are available, `WriteApiServerAuth` is refused with a reason, and a denied user is refused in the API server's own words (FR-023, FR-031–FR-035)

**Checkpoint**: Hosted and desktop both correct, with the hosted denial enforced in `filter.rs` rather than only in the UI.

---

## Phase 10: Polish & Cross-Cutting Concerns

- [X] T074 [P] Update `docs/MVP.md` with 045's entry, noting the protocol surface is additive and stays v1 (the 034 precedent)
- [X] T075 [P] Fill `specs/045-guided-oidc-setup/checklists/requirements.md` mapping every FR-001–FR-036 to the task that satisfies it, so an unimplemented requirement is visible
- [X] T076 Run `specs/045-guided-oidc-setup/quickstart.md` end to end on the dev cluster and correct any step that does not match what shipped
- [ ] T077 Run the full gate: `scripts/ci.sh all`, plus `pnpm -C ui e2e -- hosted-guided-oidc` (not `pnpm e2e:hosted`, whose `testDir` is `./tests/hosted`) and the restricted suite, and fix what 045 broke
- [ ] T078 Capture the promo screenshots of the new path per the brochure convention — **last step of the batch**, after everything else is green
- [ ] T079 Run `scripts/hygiene.sh` and clean up anything 045's work left behind before reporting done

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies.
- **Foundational (Phase 2)**: depends on Setup; **blocks every user story**. T003→T004 are sequential (same struct); T005–T007 are parallel; T008–T009 need T003–T007; T010–T011 need T008; T012–T013 need T008–T009; T014 needs T012; T015→T016 and T017 need T014 for the generated types.
- **US1 (Phase 3)**, **US2 (Phase 4)**, **US3 (Phase 5)**: all depend only on Phase 2, and are independent of each other.
- **US4 (Phase 6)**: depends on Phase 2; its *bindings* half (T044) additionally needs the save from US5 (T049), because `RoleAssignmentDraft` requires a saved `issuer_id`. The Fathom-slots half (T042) does not.
- **US5 (Phase 7)**: depends on Phase 2; T046/T049 need US2's draft sign-in (T027) for `adopt_test_session` to mean anything.
- **US6 (Phase 8)**: depends on Phase 2 and on US3's parsing (T035–T036), which the write reuses to decide the style. T057→T058→T060→T061 are sequential.
- **Hosted (Phase 9)**: depends on Phase 2 and T011; T067 needs T066; T068 needs T067.
- **Polish (Phase 10)**: depends on everything shipped. T078 is last by convention.

### Within Each User Story

- Tests precede the implementation they cover and must fail first.
- Protocol types before handlers; handlers before UI; UI before E2E.

### Parallel Opportunities

- Phase 2: T005, T006, T007 together; then T013 alongside T012; then T015 alongside T017.
- Phases 3–5 are three independent tracks once Phase 2 lands, and are the natural split for three agents.
- Every `[P]` test task in a phase can run alongside its siblings.
- Phase 9 is independent of Phases 3–8 after T011, so hosted can proceed in parallel with the desktop stories.

---

## Parallel Example: Phase 2 protocol types

```bash
# Three independent additions to src/protocol/v1/oidc.rs sections:
Task: "T005 Add IssuerDraft and DraftSignInResult"
Task: "T006 Add ApiServerAuthStyle, ApiServerAuthRead, AuthDifference, ApiServerAuthComparison"
Task: "T007 Add IssuerOrigin"
```

## Parallel Example: the three P1 stories

```bash
# After Phase 2's checkpoint, three agents, no shared files beyond GuidedIssuerPath.tsx:
Agent A: Phase 3 (US1) — discovery
Agent B: Phase 4 (US2) — draft sign-in and claims
Agent C: Phase 5 (US3) — API server read and comparison
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1: Setup.
2. Phase 2: Foundational — this is the bulk of the protocol work and blocks everything.
3. Phase 3: US1.
4. **STOP and VALIDATE**: paste a URL, see the real discovery document, confirm nothing was written.

### Incremental Delivery

1. Setup + Foundational → the Sheet opens and the protocol compiles.
2. US1 → probing works (MVP).
3. US2 → the draft sign-in works and still persists nothing.
4. US3 → the cluster comparison lands. **The path is now better than the flat form; this is the natural first release.**
5. US5 → the save, which lets the path replace the flat form.
6. US4 → the role steps and the bindings handoff.
7. Phase 9 → hosted, which can run in parallel from step 1.
8. US6 → the API server write, last because it carries the provenance work.

### Notes

- `[P]` means different files and no dependency on an incomplete task.
- Commit each coherent piece as its own signed commit once its gate is green, rather than batching.
- Run `pnpm` from `ui/`, not the repo root. Use `CARGO_BUILD_JOBS=4` when other agents are active.
- Phase 2's four `mutation.rs` matches and `filter.rs`'s `DENIED` are the two places a mistake is silent in opposite directions: the matches fail to **compile** if you forget, the `DENIED` list fails to **deny** if you forget.

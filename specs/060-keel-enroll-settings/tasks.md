---
description: "Task list for spec 060: Keel Enroll, System Settings, admin tokens, OIDC roles, System workspace and UI polish"
---

# Tasks: Keel Enroll, System Settings, admin tokens, OIDC roles, System workspace and UI polish

**Input**: Design documents from `specs/060-keel-enroll-settings/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Required. Constitution III (tested at the boundaries) is non-negotiable, and the spec's
acceptance scenarios map to the tests below.

**Gate rules for every task**:
- Run targeted tests only, one command at a time, with `CARGO_BUILD_JOBS=4`.
- Subagents never run `scripts/ci.sh`, `paws` or `dagger`.
- The main session runs the full gate once, after merging.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: the user story this task serves (US1 to US7)

## Phase 1: Setup

- [X] T001 Add `react-markdown` and `remark-gfm` (latest stable, approved) to `apps/keel/web/package.json` with `pnpm --filter @companion/keel-web add react-markdown remark-gfm`, and commit the lockfile
- [X] T002 [P] Create the empty module directories `apps/keel/web/src/ui/` and `apps/keel/web/src/features/settings/`, and `docs/guides/keel/README.md` (a short index of the Keel guides)

---

## Phase 2: Foundational (blocks every story)

The contract and the schema land first, in the main session, so the parallel agents build on one
shape.

- [X] T003 Write migration `apps/keel/server/migrations/0010_settings_system.sql` exactly per data-model.md. It covers:
  - **`instance_settings`**:
    - `id boolean primary key default true check (id)`;
    - `enroll_template text check (octet_length(enroll_template) <= 4096)`;
    - `enroll_instructions text not null default '' check (octet_length(enroll_instructions) <= 20480)`;
    - `version bigint not null default 1`;
    - `updated_by uuid references users (id) on delete set null`;
    - `updated_at timestamptz not null default now()`.
  - **`host_enrollments`**: adds `rolled_from uuid references host_enrollments (id)`.
  - **`users`**: adds `role_source text not null default 'keel' check (role_source in ('keel','idp'))`
    and `idp_role text check (idp_role in ('owner','admin','user'))`.
  - **`workspaces`**: adds `system boolean not null default false`, plus
    `create unique index workspaces_one_system on workspaces (system) where system`.
  - **`system_config`**: `id boolean pk default true check (id)`, `workspace_id uuid not null references workspaces (id)`, `allowed_profiles text[] not null` (1 to 16 entries), `version bigint not null default 1`, `updated_by`, `updated_at`.
  - **Grants**: `keel_app` gets `select, insert, update` on the new tables, plus `delete` on `system_config` only.
- [X] T004 [P] Add a pure `enroll_template` module to `crates/keel-core/src/enroll_template.rs`, exported from `crates/keel-core/src/lib.rs`:
  - `validate(&str) -> Result<(), TemplateError>`, with the codes `missing_token`, `unknown_placeholder` and `too_long` (more than 4,096 bytes);
  - `render(&str, &Values) -> String`, single pass (FR-010);
  - `DEFAULT_TEMPLATE`;
  - a test that loads `specs/060-keel-enroll-settings/contracts/template-vectors.json` and checks every `render`, `single_pass`, `refuse` and `too_long` case.
- [X] T005 [P] Extend `crates/keel-core/src/authz.rs`:
  - add `Subject.system_workspace: Option<WorkspaceId>` and `Refusal::OwnerOnly` (`code() = "owner_only"`);
  - make `require` refuse non-owners on the System workspace, before the instance-admin bypass;
  - add unit tests: an owner is allowed, an admin is refused, a user with a grant is refused, other workspaces are unchanged.
- [X] T006 Add every command and event from `specs/060-keel-enroll-settings/contracts/protocol.md` to `crates/keel-protocol/src/{cmd,event}.rs`, or the module that holds hosts, users and workspaces. That is:
  - **new commands**: `GetSettings`, `PreviewSettingsUpdate`, `ListEnrollments`, `PreviewEnrollmentRevoke`, `PreviewEnrollmentRoll`, `ListAllApiTokens`, `GetSystem`, `PreviewSystemConfigure`, `PreviewSystemRemove`;
  - **new events**: `Settings`, `RoleMappingInfo`, `Enrollments`, `EnrollmentInfo`, `EnrollmentState`, `AllApiTokens`, `AdminApiTokenInfo`, `ApiTokenState`, `System`;
  - **changed events**: `HostEnrollment` gains `snippet`, `instructions` and `instructions_unavailable`; `UserInfo` gains `role_source`; `WorkspaceInfo` gains `system`.

  Add round-trip serde tests.
- [X] T007 Regenerate the TS bindings with `make keel bindings`, and commit `apps/keel/web/src/protocol/generated/`. Depends on T006.
- [X] T008 Make `dispatch.rs` compile with every new command routed to a stub that returns `OpError { code: "not_implemented" }`. Add the new event fields, with defaults, at the existing construction sites in `apps/keel/server/src/{hosts,governance,identity}.rs`. Then run `cargo build -p keel-server`.
- [X] T009 Add `AppState.system_workspace: RwLock<Option<Uuid>>`, loaded at startup from `system_config` in `apps/keel/server/src/serve.rs`. Thread it into the `Subject` built for every caller (`apps/keel/server/src/governance.rs`).

**Checkpoint**: The workspace builds, the bindings are fresh, and the migration applies on the test database.

---

## Phase 3: User Story 1 - Enrol a host from an Enroll button (P1) 🎯 MVP

**Goal**: An owner with no team grants enrols a host from a dialog that shows the token, a correct snippet and the instructions.

**Independent test**: quickstart scenarios 1 and 2.

### Tests

- [X] T010 [P] [US1] Write a server DB test in `apps/keel/server/tests/enroll_snippet.rs`. It checks that:
  - `HostEnrollment.snippet` renders the default template with the real token when there is no settings row;
  - with a saved template, it renders that template;
  - `instructions_unavailable` is true when the settings read fails.
- [X] T011 [P] [US1] Write a Playwright spec, `apps/keel/web/tests/e2e/enroll.spec.ts`. It checks that:
  - an owner without grants sees Enroll and every workspace;
  - the token and snippet show once;
  - closing warns;
  - a user without operate sees no button.

### Implementation

- [X] T012 [US1] In `apps/keel/server/src/hosts.rs` `create_enrollment`: load `instance_settings` (falling back to defaults on error, with `instructions_unavailable = true`), and render the snippet with `keel_core::enroll_template::render`. The values are `%t` the token, `%n`, `%p` comma-joined, `%w` the slug, `%s` from `runner_host_names` and `runner_listen`, `%u` the public URL, and `%e` the RFC 3339 expiry. Never log the token.
- [X] T013 [P] [US1] Build the shared primitives `apps/keel/web/src/ui/{Dialog,CopyBlock}.tsx`, on radix-ui and the theme tokens in `index.css`. `CopyBlock` takes a masked mode with a reveal, and a copied state.
- [X] T014 [P] [US1] Write `apps/keel/web/src/features/settings/Markdown.tsx`, lazy-loaded. It uses react-markdown and remark-gfm, plus a remark plugin that turns `html` nodes into `text`. `a` gets `target=_blank rel="noopener noreferrer"`; `img` renders as a link. Add a Vitest in `apps/keel/web/tests/unit/markdown.test.tsx`.
- [X] T015 [US1] Write `apps/keel/web/src/features/hosts/EnrollDialog.tsx`:
  - **step 1**: workspace, name and profiles, then preview and confirm with step-up;
  - **step 2**: the token, the snippet, the instructions and an expiry countdown, plus a warning on close.
- [X] T016 [US1] In `apps/keel/web/src/features/hosts/HostsPage.tsx` and `store.ts`:
  - the picker reads `ListWorkspaces` instead of `me.grants` (R6);
  - add the Enroll button in the header, shown when the caller can operate somewhere;
  - remove the inline enrol form, the token box and `tokenArg` (FR-006).

**Checkpoint**: US1 works end to end with the default template.

---

## Phase 4: User Story 5 - Instance roles from the identity provider (P1)

**Goal**: Roles follow the IdP mapping. The first-user race stays closed, and the last owner is protected.

**Independent test**: quickstart scenarios 9 to 11.

### Tests

- [X] T017 [P] [US5] Add web-auth unit tests in `crates/web-auth/src/oidc.rs`: a dotted path (`realm_access.roles`) reads a nested array; a single segment behaves as before; a non-array leaf is treated as missing.
- [X] T018 [P] [US5] Add config tests in `apps/keel/server/src/config.rs`, one for each refusal in `contracts/config.md`. Each asserts that the error names the setting.
- [X] T019 [P] [US5] Add server DB tests in `apps/keel/server/tests/oidc_roles.rs`:
  - concurrent first sign-ins make exactly one owner (FR-023);
  - the highest mapped role wins;
  - a change bumps the epoch and audits `user_role` with reason `identity_provider`;
  - `NotMapped` writes no row and audits `role_not_mapped`;
  - the last-owner guard keeps owner and sets `idp_role`, with reason `idp_names_no_owner`;
  - a non-OIDC provider is refused when a match is required;
  - `PreviewUserRole` on an idp row is refused with `role_managed_by_idp`.

### Implementation

- [X] T020 [US5] In `crates/web-auth/src/oidc.rs`, teach the groups-claim lookup (around line 441) dotted paths: walk objects segment by segment, and require a string array at the leaf.
- [X] T021 [US5] In `apps/keel/server/src/config.rs`, add `RoleMapping { claim, owner, admin, user, required }` from `KEEL_OIDC_ROLE_*` and the `--oidc-role-*` flags, with the validation from `contracts/config.md`. In `apps/keel/server/src/serve.rs` (around line 444), set `ClaimsConfig.groups` to the claim, `groups_prefix` to `""`, and `groups_required` to `required`.
- [X] T022 [US5] In `apps/keel/server/src/identity.rs` `sign_in`, inside the `SIGN_IN_LOCK` advisory lock, apply the mapping per research R7. Add `SignInError::NotMapped`, and set `role_source` and `idp_role`. In `apps/keel/server/src/signin.rs`, pass the groups through and audit refusals with `role_not_mapped`.
- [X] T023 [US5] In `apps/keel/server/src/governance.rs`, refuse `PreviewUserRole` for idp-managed users, and fill `UserInfo.role_source`.
- [X] T024 [P] [US5] In `apps/keel/web/src/features/users/UsersPage.tsx`, show an "Identity provider" tag and no role control for idp rows.
- [X] T025 [P] [US5] Write `docs/guides/keel/oidc-roles.md`: the Keycloak setup, the env vars, and what owners see.

**Checkpoint**: US5 tests pass.

---

## Phase 5: User Story 2 - Snippet and instructions in System Settings (P2)

**Goal**: Admins edit the template and instructions with a live preview. Saving is audited, needs step-up and is versioned.

**Independent test**: quickstart scenarios 3 to 5.

### Tests

- [X] T026 [P] [US2] Write a server DB test in `apps/keel/server/tests/settings.rs`:
  - an admin can get the settings, and a user is denied;
  - the update previews with step-up;
  - the save audits `settings_update` with before and after;
  - a stale version gives `settings_conflict`;
  - an invalid template is refused with its code;
  - the first save inserts the row.
- [X] T027 [P] [US2] Write a Vitest in `apps/keel/web/tests/unit/template.test.ts`, running `features/settings/template.ts` against `contracts/template-vectors.json`.
- [X] T028 [P] [US2] Write a Playwright spec, `apps/keel/web/tests/e2e/settings-enrollment.spec.ts`: inline errors, the live preview, save with step-up, the conflict banner, and `<script>` shown as text.

### Implementation

- [X] T029 [US2] Write `apps/keel/server/src/settings.rs`:
  - `GetSettings` returns `Settings`, including `runner_address`, `public_url`, `role_mapping` and `warnings` (`idp_names_no_owner` while no active user has `idp_role = 'owner'` under a mapping);
  - `PreviewSettingsUpdate` follows the `pending`/`preview` pattern with `step_up: true`, re-checks admin inside the op, runs `update … where version = $n` (or inserts the first row), and audits it.

  Wire both in `dispatch.rs`.
- [X] T030 [P] [US2] Write `apps/keel/web/src/features/settings/template.ts`: the TypeScript twin of validate and render, about 40 lines.
- [X] T031 [P] [US2] Build the shared primitives `apps/keel/web/src/ui/{Tabs,PageHeader,LoadState,EmptyState}.tsx`.
- [X] T032 [US2] Write `apps/keel/web/src/features/settings/SettingsPage.tsx`, routed at `/settings/:tab` with role-gated tabs, plus `EnrollmentTab.tsx` (the template editor with the placeholder reference and preview, the instructions in Write/Preview tabs, Save, Restore default and the conflict banner) and `store.ts`.
- [X] T033 [US2] Add System Settings, pinned to the bottom of the nav for owners and admins, in `apps/keel/web/src/app/{nav.ts,Shell.tsx,routes.tsx}`, as a lazy route. Add a Vitest for tab visibility in `apps/keel/web/tests/unit/settings-tabs.test.ts`.
- [X] T034 [P] [US2] Write `docs/guides/keel/enrollment-template.md`: the placeholders, the default, and examples.

---

## Phase 6: User Story 3 - Revoke and roll enrolment tokens (P2)

**Goal**: Admins see every enrolment token, can revoke the pending ones, and can roll any that isn't used.

**Independent test**: quickstart scenarios 6 and 7.

### Tests

- [X] T035 [P] [US3] Write a server DB test in `apps/keel/server/tests/enrollments.rs`. It covers:
  - the state derivation for all five states;
  - a revoked token refused at enrolment exactly as a burned one is;
  - revoke and roll refused on used tokens (FR-019);
  - roll, which creates `rolled_from` and two audit rows, and re-checks the name;
  - that no token or hash appears in the audit context;
  - that non-admins are denied.
- [X] T036 [P] [US3] Write a Playwright spec, `apps/keel/web/tests/e2e/settings-tokens.spec.ts`: filter, revoke and roll for enrolment tokens (the new token shows once), and admin revoke of another user's API token.

### Implementation

- [X] T037 [US3] Write `apps/keel/server/src/enrollments.rs` with `ListEnrollments`, `PreviewEnrollmentRevoke` and `PreviewEnrollmentRoll`, per research R4. Roll reuses `hosts::check_enroll` and `create_enrollment`, so it returns a `HostEnrollment` with a snippet. Wire them in `dispatch.rs`.
- [X] T038 [P] [US3] Build the shared primitive `apps/keel/web/src/ui/DataTable.tsx`: responsive, so it becomes a card list under 640 px, with a filter slot and row actions.
- [X] T039 [US3] Write `apps/keel/web/src/features/settings/TokensTab.tsx`, the enrolment section: state filter, revoke, and roll, which opens the `EnrollDialog` result step.

---

## Phase 7: User Story 4 - Revoke any user's API tokens (P3)

**Goal**: Admins list and revoke every API token, with step-up when the token isn't theirs.

**Independent test**: quickstart scenario 8.

- [X] T040 [P] [US4] Write a server DB test in `apps/keel/server/tests/admin_api_tokens.rs`: the list holds every holder for admins and is denied to users; revoking someone else's token needs step-up and audits `by_admin: true`; revoking your own is unchanged.
- [X] T041 [US4] Implement `ListAllApiTokens`, and step-up plus `by_admin` in `PreviewApiTokenRevoke`, in `apps/keel/server/src/governance.rs` and `api.rs`.
- [X] T042 [US4] Add the API tokens section to `apps/keel/web/src/features/settings/TokensTab.tsx`.

---

## Phase 8: User Story 7 - The System workspace (P3)

**Goal**: Owners run the infra Ansible against Keel's host from an owner-only workspace that is invisible to everyone else.

**Independent test**: quickstart scenarios 12 to 15.

### Tests

- [X] T043 [P] [US7] Write a server DB test in `apps/keel/server/tests/system_workspace.rs`. It checks that:
  - configure creates `keel-system` and reuses it the second time;
  - `keel-system` is reserved for workspace create;
  - an admin is denied `owner_only` on ListHosts and run commands;
  - lists filter the workspace out for non-owners (workspaces, audit, enrolments, API tokens);
  - grants and API tokens are refused on it;
  - a disallowed profile gives `profile_not_allowed`;
  - remove revokes the hosts, disables the bindings, archives the workspace, deletes the config, audits it, and keeps the history;
  - a stale version gives `system_conflict`.
- [X] T044 [P] [US7] Write runner unit tests in `apps/keel/runner/src/` for the undelivered spool: the write is atomic with mode 0600, replay runs oldest first, the file is deleted on acknowledgement, and an old file expires.
- [X] T045 [P] [US7] Write a server DB test in `apps/keel/server/tests/late_result.rs`: a `runner_lost` row takes a late `Finished` and audits `run_result_late`; a duplicate is ignored; a frame from a foreign host is ignored.
- [X] T046 [P] [US7] Write a Playwright spec, `apps/keel/web/tests/e2e/settings-system.spec.ts`: an owner sets it up, binds and sees the tab; an admin sees no tab and no workspace.

### Implementation

- [X] T047 [US7] Write `apps/keel/server/src/system.rs` with `GetSystem`, `PreviewSystemConfigure` and `PreviewSystemRemove`, per `contracts/system-workspace.md`. Update `AppState.system_workspace` on commit, and wire them in `dispatch.rs`.
- [X] T048 [US7] Filter the System workspace out for non-owners, and refuse the slug `keel-system`, in `apps/keel/server/src/governance.rs` (`list_workspaces`, workspace create, grant add, API token create), `audit.rs` (list) and `enrollments.rs`. Check `allowed_profiles` in `hosts.rs` `check_enroll`.
  - Done: `ListEnrollments` (`enrollments::rows`) and `ListAllApiTokens` (`api::all_tokens`) take `system::hide_system`, and revoke/roll there treat System rows as missing for non-owners. Not done: the audit list, because the server has no audit-list command yet; whoever adds it must apply `hide_system`.
- [X] T049 [US7] Add the undelivered spool and replay after `Hello` in `apps/keel/runner/src/`, per `contracts/runner-replay.md`.
- [X] T050 [US7] Accept a late `Finished` in `apps/keel/server/src/runner_listener.rs` and `runs.rs`, per `contracts/runner-replay.md`.
- [X] T051 [US7] Write `apps/keel/web/src/features/settings/SystemTab.tsx`. When not configured, it shows an empty state with Set up. When configured, it shows the profiles, the embedded binding form, the host, recent runs, and Remove with a typed confirmation.
- [X] T052 [P] [US7] Write `docs/guides/keel/system-workspace.md`.

---

## Phase 9: User Story 6 - The polish pass (P2, after the web work merges)

**Goal**: Every section reads as one product, at 360 px, in both themes.

**Independent test**: quickstart scenario 16.

- [X] T053 [US6] Migrate every remaining section in `apps/keel/web/src/features/*` (workspaces, bindings, inventory, runs, reviews, audit, users and hosts) to the `src/ui/` primitives.
- [X] T054 [US6] Run the `impeccable` skill over `apps/keel/web` (critique, then polish, then harden), and apply its findings in `apps/keel/web/src/**`.
- [X] T055 [US6] Write a Playwright spec, `apps/keel/web/tests/e2e/polish-screens.spec.ts`: every section at 360 and 1280 px, in light and dark, asserting there is no horizontal scroll (`scrollWidth <= clientWidth`).

---

## Phase 10: Polish and cross-cutting

- [X] T056 Merge the agent branches into `main`, then remove their worktrees and branches.
- [X] T057 Run the full gate once, with `scripts/ci.sh`, in the main session. Report the wall time, the slowest steps, the delta and one speed-up idea.
- [X] T058 Run `scripts/hygiene.sh`, then `docker builder prune --filter until=24h` if any images were built.
- [X] T059 Update `specs/060-keel-enroll-settings/spec.md` status to Implemented, and tick this file.

---

## Dependencies and execution order

- **Phase 1 → Phase 2**: these block everything. T006, T007 and T008 run in sequence; T003, T004 and T005 run in parallel.
- **After Phase 2**, four parallel agents in worktrees:
  - **A, server settings and tokens**: T010, T012, T026, T029, T035, T037, T040, T041.
  - **B, OIDC roles**: T017 to T023, and T025.
  - **C, System workspace and runner replay**: T043 to T045, T047 to T050, and T052.
  - **D, web**: T011, T013 to T016, T024, T027, T028, T030 to T034, T036, T038, T039, T042, T046 and T051.
- **US6 (Phase 9)** runs after D merges.
- **Phase 10** runs last.
- **Shared files**: `dispatch.rs` and `governance.rs` are touched by A, B and C. Keep each agent's edits to separate match arms and functions, and resolve on merge in the main session.

## Implementation strategy

- **MVP**: Phases 1 and 2, then US1. That gives the owner a working Enroll with a correct snippet.
- **Next**: US5, so the second sign-in doesn't become owner (P1); then US2 and US3; then US4 and US7; then US6.

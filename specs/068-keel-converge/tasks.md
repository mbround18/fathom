---
description: "Task list for spec 068, Keel host provisioning and convergence"
---

# Tasks: Keel host provisioning and convergence

**Paths**:

- `core/` means `crates/keel-core/src/`.
- `proto/` means `crates/keel-protocol/src/`.
- `exec/` means `crates/keel-exec/src/`.
- `trust/` means `crates/keel-trust/src/`.
- `forge/` means `crates/keel-forge/src/`.
- `server/` means `apps/keel/server/`.
- `runner/` means `apps/keel/runner/`.
- `web/` means `apps/keel/web/`.

**Lanes** (for splitting across 3–4 agents): each task is tagged **[core]**, **[server]**,
**[runner]** or **[web]**. `proto/`, `exec/`, `trust/` and `forge/` count as **[core]**. Within a
phase, tasks in different lanes marked [P] can run at once; tasks in one lane run in order.

**Gates for agents**:

- Run only targeted checks for what you touched, one command at a time, with
  `CARGO_BUILD_JOBS=4`, logging to files. For example `cargo test -p keel-core converge`,
  `cargo test -p keel-server --test converge_lock`, `cargo test -p keel-runner --test check_mode`,
  `pnpm --filter @companion/keel-web test`, or one Playwright project (`--project=s068`).
- **Never** run `scripts/ci.sh`, `paws`, `dagger` or the full Playwright suite. The main session
  runs the full gate once, on the combined branch.
- Server integration tests need `KEEL_TEST_DATABASE_URL` and fail under `CI=1` without it.
  Report the `running N tests` count for each test file you ran.
- Delete any kind cluster or compose project you start before reporting.

**Tests**: written first in each story and seen failing (constitution Principle III).

**External work**: tasks marked **(../ansible)** are contract deltas for the ansible session.
Nothing in `../ansible` is edited from this repo.

**Migration**: `server/migrations/0019_converge.sql` (0015, 0017 and 0018 are 067's). If a
0019 exists on main at merge time, renumber before any shared database applies it.

## Phase 1: Setup

- [X] T001 [server] Create `server/migrations/0019_converge.sql` with a header comment only, and
  a test in `server/tests/migrate.rs` that migration versions are unique and contiguous except
  the documented 0015/0017 reservations.
- [X] T002 [P] [web] Add Playwright project `s068` (`testDir: './tests/e2e/s068'`) to
  `web/playwright.config.ts`, update its header list, create `web/tests/e2e/s068/` and an empty
  `web/tests/support/fake-converge.ts`.
- [X] T003 [P] [server] Runtime flag `KEEL_CONVERGE` (default off) in `server/src/config.rs`,
  reported in `SystemInfo.flags`; with it off, every converge command answers `feature_disabled`.
- [X] T004 [P] [server] Fixture repo `server/tests/fixtures/converge-repo/` (manifest with
  `debian`, `keel-node` recipe, `k3s-master` covering `k3s-audit`, a `choice` param, an `env`
  binding; playbooks that write a marker per job) and a helper in `server/tests/common` to bind it.

## Phase 2: Foundational (blocks every story)

- [X] T005 [P] [core] Tests first in `core/converge.rs` (`#[cfg(test)]`): `expand` order
  (includes depth-first, then jobs), first position wins, cycle error names the cycle, unknown
  include, > 32 steps; proptest: every job in the output is reachable and appears once.
- [X] T006 [core] `Profile` gains `includes: Vec<String>`, `jobs: Vec<String>` (`#[serde(default)]`)
  in `core/manifest.rs`; implement `converge::expand` (pure) and the "no recipe ⇒ jobs naming the
  profile, by name" fallback.
- [X] T007 [P] [core] Tests first: `coverage` closure (transitive, self-cover and cycle refused),
  `host_plan` drops covered steps (k3s-master + k3s-audit ⇒ k3s-master), `covers()`.
- [X] T008 [core] `Job` gains `covers: Vec<String>`; implement `converge::coverage`, `host_plan`,
  `covers`.
- [X] T009 [P] [core] `PolicyMatch.check: Option<bool>` in `core/policy.rs` and
  `converge::combine_policies` (any approval, max count, any four-eyes, min TTL), with tests.
- [X] T010 [core] `manifest::validate` runs `expand` on every profile, `coverage`, validates
  `groups:` targets, and returns the new error codes (contracts/manifest.md). Test: every
  manifest under `server/tests/repos/` and the current `../ansible` manifest copied as a fixture
  validate exactly as before (SC-006).
- [X] T011 [P] [core] Move the reserved-name rule from `runner/src/executor/secrets.rs:71` to
  `core/env_names.rs` (runner uses it from there); parse and validate `jobs.<j>.env` with tests.
- [X] T012 [P] [core] `proto/converge.rs` and the changed types in contracts/protocol.md
  (`RunKind::Converge`, new run/host states and reasons, `CredentialKind::EnvValue`,
  `PreviewRunStart.params/check`, `BindingView`, `HostView` additions), serde round-trip tests,
  regenerate ts-rs bindings.
- [X] T013 [P] [core] `exec/frames.rs`: `Assign.check` (skipped when false), `SecretItem` kind
  `env_var { name }`, capability constant `manifest-v068`; round-trip tests and a test that a
  frame with `check: true` fails to parse under the old struct shape (fail closed).
- [X] T014 [server] Fill `0019_converge.sql` per data-model.md: `runs` and `run_hosts` column
  and check changes, `host_enrollments.auto_converge`, `converge_hosts` with
  `converge_hosts_one_holding`, `converge_steps`, `profile_suggestions`,
  `binding_required_checks`, `bindings.checks_timeout_secs`, `run_check_states`, grants.
- [X] T015 [server] Runner floor: sync records whether a binding's manifest uses any 068 key and
  computes `runners_refusing_manifest` from `hosts.runner_version`/capabilities; `dispatch_ready`
  refuses a host whose session lacks `manifest-v068` for such a binding with `runner_too_old`.
  Test in `server/tests/bindings.rs`. (Needs 067 `Hello.capabilities`; if absent, add the field
  with 067's shape.)

**Checkpoint**: core lane done ⇒ server and runner lanes can start their stories.

## Phase 3: User Story 1, converge a host to its profiles (P1) 🎯 MVP

**Goal**: one approval runs every host's ordered steps under a per-host lock; a failure skips
that host's rest; retry resumes.
**Independent test**: `cargo test -p keel-server --test converge` and `--test converge_lock`.

- [ ] T016 [P] [server] Tests first, `server/tests/converge.rs`: preview lists host × step with
  `via`, params and credential names; one approval queues each host's first step; steps run in
  order; a failed step skips the rest on that host (`dependency_failed`) while another host
  finishes; retry creates a converge with `retry_of` of only the not-succeeded steps at the same
  commit; signed mode: one child run per job, each with its review group.
  Partial: all but signed mode are covered (10 tests); the signed-mode case waits on T018.
- [X] T017 [P] [server] Tests first, `server/tests/converge_lock.rs`: a path run created between
  step 1 and step 2 waits until the chain ends; two converges cannot both hold a host (unique
  index); a `lost` step releases the hold; a replayed `Finished` on a lost row does not advance;
  restart with a hold and no live step is reconciled.
- [ ] T018 [server] `server/src/converge.rs` `preview_converge`: resolve targets, pin commit and
  inventory, `host_plan` per host, refusals (outside ceiling, runner too old, revoked),
  `combine_policies`, signed-mode `review::early_check` per child, `replaces`/`blocked_by`.
  OTel span `converge.plan` (hosts, steps).
  Partial: signed-mode bindings are refused (`signed_mode_unsupported`); `review::early_check` per child is not built.
- [X] T019 [server] `create_converge`: one transaction inserting the parent run, one child run
  per distinct job (`parent_run_id`, `Mirror::pin` per child), `converge_hosts` (`waiting`),
  `converge_steps`, `run_hosts` (`blocked`); audit `converge.create`; superseding an unapproved
  converge sets it `cancelled/replaced` with `converge.replace`.
- [X] T020 [server] Trigger `converge_advance` in `0019_converge.sql` (data-model.md §Trigger),
  plus a SQL-level test in `server/tests/converge_lock.rs` driving each terminal transition.
- [X] T021 [server] `server/src/queue.rs` `dispatch_ready`: the `converge_hosts` `not exists`
  filter and `waiting → holding` on claiming a converge step, same transaction;
  `recover_in_flight` reconcile pass; span `converge.step` (job, outcome) on finish.
- [X] T022 [server] `server/src/approvals.rs`: approving a converge parent moves each host's
  position-0 row `blocked → queued`; deny, expire, cancel cascade to children and release holds;
  `try_finish` ends the parent when every `converge_hosts` row is terminal.
- [X] T023 [server] Retry: `PreviewConvergeRetry`/`ConfirmConvergeRetry` (same commit and
  inventory; `replan: true` re-plans at head), through the approval policy (see Q3).
- [X] T024 [server] Wire commands and events in `server/src/api.rs`: `PreviewConverge`,
  `ConfirmConverge`, `CancelConverge`, `GetConverge`, retry, `ConvergeChanged`; authz Operate on
  every profile of every host; `feature_disabled` when the flag is off.
  Commands route through `dispatch.rs`; Operate is checked on the binding's workspace, not per profile.
- [X] T025 [P] [runner] Test in `runner/tests/`: a recipe manifest parses; a step whose job's
  profiles are outside the ceiling is refused as today (FR-017, no code change expected).
- [X] T026 [P] [web] `web/src/features/converge/` store and `ConvergePreview` dialog (matrix,
  `via`, params, credential names, combined policy, signed review list, refusals, replaces).
- [X] T027 [web] `ConvergePage` (live matrix, cell → child-run log, Cancel, Retry failed), routed
  from `/runs/:id` for kind `converge`; runs list hides child runs by default.
- [X] T028 [web] Hosts: converge column, bulk "Converge…"; host page Converge button and last
  converge summary.
- [X] T029 [P] [web] `fake-converge.ts` and `s068/converge-preview.spec.ts`,
  `s068/converge-page.spec.ts` (360 px and 1280 px, keyboard confirm).
- [X] T030 [web] Vitest for the matrix/state derivations in `features/converge/`.
- [ ] T031 [web] Impeccable pass over the preview, converge page and Hosts column, and the
  screens around them; fix what it finds.

## Phase 4: User Story 2, new hosts converge on enrollment (P1)

**Independent test**: `cargo test -p keel-server --test converge enroll`.

- [ ] T032 [P] [server] Tests first in `server/tests/converge.rs` (`enroll_*`): enroll ⇒ converge
  by `system:enroll`, audited; `auto_converge = false` ⇒ none; flag off ⇒ none; a plan failure
  does not undo enrollment.
- [ ] T033 [server] `auto_converge` on enrollment create/view (`server/src/enrollments`/settings);
  `converge::on_enrolled` called after `runner_listener.rs` `enrol()` commits.
- [ ] T034 [server] Hook `on_enrolled` into join-request acceptance. **Blocked on 067 D3
  (T058).**
- [ ] T035 [P] [web] "Converge after enrollment" checkbox with the planned steps in Settings →
  Enrollment and Add host; `s068/enroll-auto-converge.spec.ts`.

## Phase 5: User Story 3, edit a host's profiles, with suggestions (P1)

**Independent test**: `cargo test -p keel-server --test host_profiles`.

- [X] T036 [P] [core] `converge::suggest` tests and implementation (adds from groups; removes
  only for profiles a mapping could produce; dismissed suggestions return only when groups
  change).
- [ ] T037 [P] [server] Tests first, `server/tests/host_profiles.rs`: step-up required; cert
  principals match after add/remove; audit in the same transaction; suggestions computed at sync
  and never assigned; dismiss.
- [ ] T038 [server] `PreviewHostProfiles`/`ConfirmHostProfiles` in `server/src/hosts.rs`:
  `host_profiles` with `assigned_by`, `hosts::issue_cert` re-issue, `host.profiles_set`,
  `HostProfilesChanged`.
- [ ] T039 [server] `server/src/sync.rs`: refresh `profile_suggestions` from `groups:` and the
  pinned inventory's groups; `ProfileSuggestionsChanged`; `DismissProfileSuggestion`; span
  `profile.suggest`.
- [ ] T040 [P] [web] Host page Profiles card (chips, ceiling, outside-ceiling marker, editor with
  preview and step-up), suggestions list, Hosts suggestion dot; `s068/host-profiles.spec.ts`,
  `s068/suggestions.spec.ts`.
- [ ] T041 [web] Impeccable pass over the Profiles card and suggestions, and the host page around
  them.

## Phase 6: User Story 4, narrow jobs and path triggers without double work (P2)

**Independent test**: `cargo test -p keel-server --test coverage`.

- [ ] T042 [P] [server] Tests first, `server/tests/coverage.rs`: a path run of `k3s-audit` ends
  `covered` (linked, audited) on a host with a waiting/holding `k3s-master` converge at the same
  or a descendant commit; queues on other hosts; an older converge commit does not cover.
- [ ] T043 [server] `server/src/runs.rs` `create_run`: coverage lookup via
  `converge_hosts`/`converge_steps` and `converge::covers`, mirror ancestry check, insert
  `covered` rows with `covered_by`, audit `run_host.covered`.
- [X] T072 [P] [core] Tests first, then `converge::narrow_by_vars(changed: &[Path], matched_paths, inventory) -> Option<BTreeSet<HostName>>` in `core/converge.rs` (FR-020): `None` when any matched change is not a group/host vars file or is `group_vars/all.*`; otherwise the hosts in the named groups (children included) and named hosts. Cover `.yml`/`.yaml`, directory-form `group_vars/<g>/`, child groups, and an unknown group (narrows to no hosts → no run, audited `run.skipped: no_hosts`).
- [ ] T073 [server] `server/src/runs.rs` `on_push`: intersect the job's host set with `narrow_by_vars` for path-triggered runs only; record the narrowing reason on the run (`narrowed_by: ["group_vars/keel_servers.yml"]`) and show it in the run header. Test in `server/tests/coverage.rs`.
- [ ] T044 [P] [web] "covered by converge" in run host rows; `s068/covered.spec.ts`.
- [ ] T045 (../ansible) Hand contracts/ansible.md §2–§3 (narrow jobs, covers, the paths
  recommendation) to the ansible session.

## Phase 7: User Story 5, check-mode runs (P2)

**Independent test**: `cargo test -p keel-server --test check_mode` (SC-004).

- [X] T046 [P] [core] `trust/ledger.rs` `Outcome::Checked`: tests that it does not move the base
  and that a replayed run id is still refused; implement.
- [X] T047 [P] [runner] Tests first, `runner/tests/check_mode.rs`: `check: true` adds
  `--check --diff`; signed mode writes `Checked`; base unchanged.
- [X] T048 [runner] Implement in `runner/src/executor/mod.rs` and `process.rs`.
- [ ] T049 [P] [server] Tests first, `server/tests/check_mode.rs`: check run and check converge
  write no `host_revisions`, drift or rollback point; policy with `check: false` doesn't match a
  check run; listed with `check`.
- [ ] T050 [server] `runs.check_mode` through `create_run`/`create_converge`, `Assign.check`,
  `finished()` skips revision writes; policy matching on `check`.
- [ ] T051 [P] [web] Check toggles (Start dialog, converge preview), `check` badges in every run
  list; `s068/check-mode.spec.ts`.

## Phase 8: User Story 6, credentials as environment variables (P2)

**Independent test**: `cargo test -p keel-server --test env_canary` (SC-005).

- [X] T052 [P] [runner] Tests first, `runner/tests/env_vars.rs`: `env_var` item lands in
  `ansible-playbook`'s env after `env_clear`; reserved names refused (`env_refused`); value not in
  the runner log.
- [X] T053 [runner] Implement in `runner/src/executor/secrets.rs` using `keel_core::env_names`.
  Note: missing `env_var` credentials are not counted by core `missing_credentials`; the runner fails the run itself ("env credentials not sent"). Refusal code `env_refused` is new in keel-exec.
- [X] T054 [P] [server] Tests first, `server/tests/env_canary.rs`: a canary credential via
  `env:`; assert it appears in no log, event, audit row, or text/jsonb/bytea column outside the
  sealed credential store.
  Note: also covers a missing and a wrong-kind `env:` credential (run fails `credential_unavailable` before the host sees an `Assign`). The column sweep covers every text/varchar/json/jsonb/bytea/array column of every table in the test schema; run logs live in `run_log_chunks.bytes`, so they are swept too.
- [X] T055 [server] `queue.rs` `secrets()` resolves every credential through `SecretResolver`
  (`core/ports.rs:97`) instead of `credentials::open`; `env_value` credential kind; `env_var`
  items; credential names (only) in the converge and run previews.
  Note: the core `SecretResolver` port is sync, so the server adapter is `credentials::Granted`: the granted rows are loaded (still sealed) once per assignment, and each value is unsealed only in `resolve`. `env:` entries must be `env_value` credentials and go out as `env_var` items with `env_name`; anything missing or of another kind fails the assignment with `credential_unavailable` (US1 leftover: core `missing_credentials` still does not count `env:`). Migration 0019 adds `env_value` to `credentials_kind_check`. Run previews end with "Credentials sent: a, b (as $VAR)."; converge plans already carry `credential_names`. A dispatch now also kicks after a converge settles and after `CancelConverge`, so a released hold sends the next run at once.
- [ ] T056 [P] [web] `env_value` kind in the credentials UI; credential names shown in previews.

## Phase 9: User Story 7, runnable only once checks passed (P3)

**Independent test**: `cargo test -p keel-server --test required_checks`.

- [X] T057 [P] [core] `Forge::commit_statuses` in `core/ports.rs`; Forgejo and GitHub adapters in
  `forge/` with recorded-fixture tests (`cargo test -p keel-forge statuses`); `Unsupported` for
  others.
  Note: signature is `commit_statuses(sha)` (adapters are already repo-bound, `RepoRef` lives in keel-forge); core state enum is `CommitStatusState` (no `Missing`: the server derives it); GitHub reads one page of 100 check runs.
- [ ] T058 [P] [server] Tests first, `server/tests/required_checks.rs`: `waiting_checks →
  queued` on all success; `checks_failed`; `checks_timeout`; preview refused on an unsupported
  forge; converges and runs both gated.
- [ ] T059 [server] `server/src/checks.rs`: required-check commands (step-up), `waiting_checks`
  at create, poller on the sync cadence, `run_check_states`, `RunChecksChanged`, span
  `checks.wait`.
- [ ] T060 [P] [web] Binding Required checks section; `waiting_checks` display with forge links;
  `s068/required-checks.spec.ts`.

## Phase 10: User Story 8, one-off jobs with parameters (P3)

**Independent test**: `cargo test -p keel-server --test choice_params`.

- [X] T061 [P] [core] `ParamKind::Choice` and `RepoTree::list_dirs` in `core/manifest.rs`, with
  tests for both shapes, `default`, and the `from:` limits.
  Note: `RepoTree::list_dirs` returns `Option<Vec<String>>` (default `None`, fails closed) rather than `Result<_, TreeError>`.
- [X] T062 [P] [runner] Tests first, `runner/tests/choice.rs` (re-check against the checkout,
  `choice_mismatch`) and `runner/tests/prompt.rs` (a fixture `pause` play fails in < 5 s).
  Note: the prompt fixture reads `/dev/tty` in the fake `ansible-playbook` (not a real `pause` play); the run finishes in well under 5 s, the test's hang guard is 60 s.
- [X] T063 [runner] Implement the `choice` re-check and `setsid()` in place of `setpgid(0,0)`
  (`runner/src/executor/process.rs:194`), keeping group kill.
- [ ] T064 [P] [server] Tests first, `server/tests/choice_params.rs`: allowed values at the
  pinned commit, refusal of other values, `PreviewRunStart` with params and check.
- [ ] T065 [server] `RepoTree::list_dirs` from the mirror/`repo_catalog`; `ParamView.allowed`;
  `PreviewRunStart.params`/`check` through `check_params` (`runs.rs:177`).
- [ ] T066 [P] [web] Start dialog params with a `choice` select; `s068/choice-param.spec.ts`.

## Phase 11: Polish and cross-cutting

- [ ] T067 [P] `docs/guides/keel/converge.md` (recipes, converge, profiles and suggestions,
  covers, check mode, env credentials, required checks, choice params, the runner-upgrade order),
  and a CONTRIBUTING note on the `converge_advance` trigger.
- [ ] T068 (../ansible) Hand the whole contracts/ansible.md to the ansible session in the order
  of quickstart §4 (runners upgraded, ceilings, then manifest); record its confirmation of SC-002.
- [ ] T069 [web] Impeccable pass over every 068 screen together (Hosts, host page, converge
  preview and page, Start dialog, binding page, enrollment).
- [ ] T070 Main session only: `scripts/hygiene.sh`, then the full gate once on the combined
  branch (`scripts/ci.sh`); report wall time, slowest steps and delta vs the last pass.
- [ ] T071 [web] Brochure screens last: add the converge preview, converge page and Profiles
  card to `web/tests/e2e/polish-screens.spec.ts` and regenerate.

## Dependencies & execution order

- Phase 1 → Phase 2. In Phase 2, T005–T013 ([core]) block T014–T015 only where types are used;
  T014 can start with T005.
- US1 (Phase 3) needs Phase 2. US2, US3 and US4 need US1's server tasks T018–T022. US5–US8 need
  only Phase 2 and can run in parallel with US1.
- T034 waits for 067 D3. T045/T068 wait for runners on 068 (research R4) and the ansible delta adding `debian` to the ceilings (Q1, decided 2026-11-17).
- T070 after every agent branch is merged; T071 after T069.

## Parallel opportunities

Four agents, one per lane, after Phase 2:

| Lane | Takes |
|---|---|
| core | T005–T013, then T036, T046, T057, T061 |
| server | T001, T003, T004, T014–T024, T032–T034, T037–T039, T042–T043, T049–T050, T054–T055, T058–T059, T064–T065 |
| runner | T025, T047–T048, T052–T053, T062–T063 |
| web | T002, T026–T031, T035, T040–T041, T044, T051, T056, T060, T066, T069, T071 |

The server lane is the long pole; split it in two after US1 (US2–US4 in one agent, US5–US8 in
another) if a fifth agent is available.

## Implementation strategy

MVP = Phases 1–3 with `KEEL_CONVERGE` on in dev. Then US2 + US3 (the onboarding loop, SC-001),
then US4 (SC-003), then US5–US8 in any order. The ansible delta goes last, after the runner
upgrade.

## Task count

| Phase | Tasks |
|---|---|
| Setup | 4 |
| Foundational | 11 |
| US1 | 16 |
| US2 | 4 |
| US3 | 6 |
| US4 | 4 |
| US5 | 6 |
| US6 | 5 |
| US7 | 4 |
| US8 | 6 |
| Polish | 5 |
| **Total** | **71** |

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

- [X] T016 [P] [server] Tests first, `server/tests/converge.rs`: preview lists host × step with
  `via`, params and credential names; one approval queues each host's first step; steps run in
  order; a failed step skips the rest on that host (`dependency_failed`) while another host
  finishes; retry creates a converge with `retry_of` of only the not-succeeded steps at the same
  commit; signed mode: one child run per job, each with its review group.
  Signed mode (module `signed`, 3 tests): the preview lists each step's review at the plan's one
  commit; confirming makes one child per job, all at that commit, each in `needs_review` with its
  own group (its hosts); the plan's approval is refused while in review; reviewing one step queues
  that child but nothing is dispatched until every step is reviewed; each step goes out with its
  own step's override only (the `Approvals` frame per child run id); a plan needing two approvals
  waits in `pending_approval` after one reviewer; denying one step's review denies the converge.
- [X] T017 [P] [server] Tests first, `server/tests/converge_lock.rs`: a path run created between
  step 1 and step 2 waits until the chain ends; two converges cannot both hold a host (unique
  index); a `lost` step releases the hold; a replayed `Finished` on a lost row does not advance;
  restart with a hold and no live step is reconciled.
- [X] T018 [server] `server/src/converge.rs` `preview_converge`: resolve targets, pin commit and
  inventory, `host_plan` per host, refusals (outside ceiling, runner too old, revoked),
  `combine_policies`, signed-mode `review::early_check` per child, `replaces`/`blocked_by`.
  OTel span `converge.plan` (hosts, steps).
  Signed mode: `review::early_check` runs per child (one per job, over that job's hosts, at the one
  pinned commit); `ConvergePlan.review` lists each child. `create` starts a child with groups in
  `needs_review` (its hosts' bases recorded, groups inserted, the plan's four-eyes on the child)
  and the parent with it; the parent holds its approval terms from the start (window stashed as
  `created_at` + TTL). A child's groups open only while the parent is in review (`group_ctx`
  reads the parent's state). `converge::reviewed` moves the parent on once no child is left in
  review, counting distinct reviewers across children against `approvals_needed`;
  `converge::review_denied` denies the plan and abandons its steps. `checks::release` sends a
  converge whose children wait for review to `needs_review`. The runner is unchanged: `assign`
  already sends each child's own override approvals, bound to its run id and job.
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
- [X] T031 [web] Impeccable pass over the preview, converge page and Hosts column, and the
  screens around them; fix what it finds.
  Done: phone cards show each step's state in words (icon-only before); the desktop matrix sizes
  to its steps instead of spreading across the page; the Hosts converge cell no longer wraps
  mid-phrase. The preview passed as is.

## Phase 4: User Story 2, new hosts converge on enrollment (P1)

**Independent test**: `cargo test -p keel-server --test converge enroll`.

- [X] T032 [P] [server] Tests first in `server/tests/converge.rs` (`enroll_*`): enroll ⇒ converge
  by `system:enroll`, audited; `auto_converge = false` ⇒ none; flag off ⇒ none; a plan failure
  does not undo enrollment.
  Note: four `enroll_*` tests in `tests/converge.rs` through the real `enroll_key`: a converge for that host alone (debian then keel-node), `requested_by` null, `converge.create` audited with actor `system:enroll`, queued with no approval; pending_approval under an approval policy; `auto_converge: false` and `KEEL_CONVERGE` off make none (and `EnrollmentInfo.auto_converge` reads false); a host no binding can plan for stays enrolled and `converge.enroll_skipped` is audited with each binding's refusal code.
- [X] T033 [server] `auto_converge` on enrollment create/view (`server/src/enrollments`/settings);
  `converge::on_enrolled` called after `runner_listener.rs` `enrol()` commits.
  Note: There is no enrollment-settings object: `auto_converge` lives on the token. `PreviewHostEnroll` gains `auto_converge?: boolean` (absent = true, so old clients keep converging), stored in `host_enrollments.auto_converge` (migration 0019) and shown as `EnrollmentInfo.auto_converge`; a rolled token copies the old one's choice; the host-add proposal path passes true. `converge::on_enrolled` runs after `enroll_key` commits: it tries the workspace's active bindings oldest first (by id) and makes one converge from the first whose plan gives the host a step; otherwise it audits `converge.enroll_skipped` (reason `no_plan` or `no_binding`, context per binding code/message). Errors are logged and never undo the enrollment. New audit actor `Actor::Enroll` keyed `system:enroll`. The "host page says why" part of US2 scenario 4 is only the audit entry here; nothing on `HostView` yet. `converge::create` now takes an actor and an optional user instead of a `Caller`.
- [X] T034 [server] Hook `on_enrolled` into join-request acceptance. **Blocked on 067 D3
  (T058).**
  Note (c068-server2): still blocked. `join.rs` and migration 0018 (067 D3) are absent from this
  branch. `converge::on_enrolled(app, host)` is public and treats a host with no
  `host_enrollments` row as auto-converge on, so acceptance only needs to call it after its
  transaction commits.
  Note (c068-finish): done. The choice comes from the acceptance, not the join token:
  `PreviewJoinAccept` gains `auto_converge?: boolean` (absent = true, like `PreviewHostEnroll`).
  A join token is workspace-wide and reusable, while acceptance is the per-host decision where the
  operator also picks the profiles (US2 scenario 3). The choice is recorded in the `join_accepted`
  audit context; no migration (`on_enrolled` is called directly once the decide transaction
  commits, errors logged, never undoing the acceptance). The preview summary says Keel will
  converge it when the choice is on and converge is enabled. Tests: `tests/converge.rs`
  `join_acceptance_*`. Done: a "Converge after accepting" checkbox in `JoinRequestsTab.tsx` (default on, field omitted; unticked sends `auto_converge: false`), covered in `tests/e2e/us7/join.spec.ts`.
- [X] T035 [P] [web] "Converge after enrollment" checkbox with the planned steps in Settings →
  Enrollment and Add host; `s068/enroll-auto-converge.spec.ts`.
  Done: the checkbox in the enroll dialog (default on; unticked sends `auto_converge: false`).
  Add host gets a note only, because `PreviewHostAdd` has no `auto_converge`. No wire carries
  the enrollment plan, so the checkbox names the profiles rather than listing the steps.

## Phase 5: User Story 3, edit a host's profiles, with suggestions (P1)

**Independent test**: `cargo test -p keel-server --test host_profiles`.

- [X] T036 [P] [core] `converge::suggest` tests and implementation (adds from groups; removes
  only for profiles a mapping could produce; dismissed suggestions return only when groups
  change).
- [X] T037 [P] [server] Tests first, `server/tests/host_profiles.rs`: step-up required; cert
  principals match after add/remove; audit in the same transaction; suggestions computed at sync
  and never assigned; dismiss.
  Note: three tests: preview shows add/remove, `outside_ceiling`, `principals_after` and `plan_after`, and needs step-up (a stale session without a passkey gets `PasskeyEnrolmentRequired` and nothing changes); confirm sets `host_profiles` with `assigned_by`, re-issues a certificate whose principals drop out-of-ceiling and removed profiles, audits `host.profiles_set` with the serial; refusals `no_change`, `unknown_profile`, `invalid_change`, `not_found`, `host_revoked`; suggestions from `groups:` at sync (add + remove, `HostInfo.suggestions` count), never assigned, dismiss survives a same-groups sync and returns when the groups change, accepting through the preview settles the add. "Audit in the same transaction" is by construction (one tx); the test checks the row, not a forced rollback.
- [X] T038 [server] `PreviewHostProfiles`/`ConfirmHostProfiles` in `server/src/hosts.rs`:
  `host_profiles` with `assigned_by`, `hosts::issue_cert` re-issue, `host.profiles_set`,
  `HostProfilesChanged`.
  Note: Deviations: the generic `Confirm` replaces `ConfirmHostProfiles`; the preview replies `Event::HostProfilesPreview { preview_token, preview }` before its `Preview` (like `ConvergePlan`). `plan_after` comes from the first active non-signed binding (oldest first) whose manifest gives the host a step. The certificate is re-issued in the transaction when the host has a key and the CAs are loaded, then sent as `Frame::Cert` to its live session (`Runs::send` is now `pub(crate)`); a disconnected host gets the new principals at its next renewal. The old certificate is not revoked, so a removed profile's principal lasts until that cert expires (dispatch already follows `host_profiles`). Confirm also deletes the suggestions the change settles. `HostProfilesChanged` is pushed to the workspace. New OpError codes `invalid_change`, `unknown_profile`, `too_many_profiles`, `host_revoked`.
  KRL (owner decision 2026-11-17, "revoke via KRL"; supersedes "the old certificate is not revoked" above): `hosts::settle_certs(app, host, serial)` runs after every runner session attaches (`runner_listener.rs`, `Access::Runner`, spawned so it never delays the session). It revokes the host's older unrevoked certificates whose principals differ from the one the runner authenticated with, audits `host_cert_superseded` (actor `host:<id>`, context `serials` and `by`), and rebroadcasts `Frame::Krl` through `publish_krl`. Revocation waits for proof that the runner holds the new certificate (it authenticated with it, or with a later renewal) rather than happening at issue, because there is no ack for a pushed `Cert` and a runner refused with an unexpired certificate does not fall back to bare-key renewal: revoking at issue would lock out a runner that was offline (or lost the push) until its old certificate expired. Renewals keep their principals and revoke nothing, so the KRL does not churn. A runner that comes back with a certificate older than a profile change is pushed a fresh one (`renew`, audited `host_cert_renewed`) and its old certificate is revoked at its next session with the new one. Trade-off: for a runner that stays connected, the old certificate is revoked at its next reconnect, or simply expires (`runner_cert_ttl`, 24 h by default). Test: `superseded_certificate_is_revoked_once_the_runner_holds_the_new_one` in `tests/host_profiles.rs`.
- [X] T039 [server] `server/src/sync.rs`: refresh `profile_suggestions` from `groups:` and the
  pinned inventory's groups; `ProfileSuggestionsChanged`; `DismissProfileSuggestion`; span
  `profile.suggest`.
  Note: Suggestions are refreshed after every applied sync with a valid manifest, moved or not, from the hosts' groups in the latest valid revision of each inventory the manifest's jobs use; never fails the sync. A dismissed row keeps its dismissal while its groups are unchanged (SQL equality, the same rule as `still_dismissed`); a suggestion this binding no longer makes is deleted, so leaving and rejoining a group brings it back. `ProfileSuggestionsChanged { binding_id, hosts }` is pushed only when a host's open suggestions changed. `ListProfileSuggestions` takes a required `workspace` (contract had only `host_id?`); `DismissProfileSuggestion` replies `ProfileSuggestions` for the host and audits `profile_suggestion.dismissed`. `HostInfo.suggestions` is now the open count.
- [X] T040 [P] [web] Host page Profiles card (chips, ceiling, outside-ceiling marker, editor with
  preview and step-up), suggestions list, Hosts suggestion dot; `s068/host-profiles.spec.ts`,
  `s068/suggestions.spec.ts`.
  Done: `HostProfiles.tsx` and `profiles-store.ts`, with the fake in
  `tests/support/fake-profiles.ts`. Converge being off is detected from `feature_disabled`
  (SystemInfo has no flag).
- [X] T041 [web] Impeccable pass over the Profiles card and suggestions, and the host page around
  them.
  Done: chips at 360 px keep "outside ceiling" whole; the outside-ceiling note no longer repeats
  the chips; the Hosts Profiles cell wraps between names, not inside them.

## Phase 6: User Story 4, narrow jobs and path triggers without double work (P2)

**Independent test**: `cargo test -p keel-server --test coverage`.

- [X] T042 [P] [server] Tests first, `server/tests/coverage.rs`: a path run of `k3s-audit` ends
  `covered` (linked, audited) on a host with a waiting/holding `k3s-master` converge at the same
  or a descendant commit; queues on other hosts; an older converge commit does not cover.
  Note: the same-commit and descendant cases replay the push through `runs::on_push` after the converge exists (a real push always precedes the converge it would need). Also: hosts outside the converge queue, and a cancelled converge covers nothing. `covered_by` is the converge parent run (data-model), so the protocol doc comment now says so.
- [X] T043 [server] `server/src/runs.rs` `create_run`: coverage lookup via
  `converge_hosts`/`converge_steps` and `converge::covers`, mirror ancestry check, insert
  `covered` rows with `covered_by`, audit `run_host.covered`.
  Note: a host is covered when an open converge of the same binding has it waiting/holding with a step still blocked/queued/sent/running that is the job or covers it (coverage from the manifest at the converge commit), and the converge commit is the run commit or a mirror descendant. Only path-triggered runs (`on_push`) are checked. A run whose every host is covered ends `succeeded` at once (core `summarize` already counts `covered` as success).
- [X] T072 [P] [core] Tests first, then `converge::narrow_by_vars(changed: &[Path], matched_paths, inventory) -> Option<BTreeSet<HostName>>` in `core/converge.rs` (FR-020): `None` when any matched change is not a group/host vars file or is `group_vars/all.*`; otherwise the hosts in the named groups (children included) and named hosts. Cover `.yml`/`.yaml`, directory-form `group_vars/<g>/`, child groups, and an unknown group (narrows to no hosts → no run, audited `run.skipped: no_hosts`).
- [X] T073 [server] `server/src/runs.rs` `on_push`: intersect the job's host set with `narrow_by_vars` for path-triggered runs only; record the narrowing reason on the run (`narrowed_by: ["group_vars/keel_servers.yml"]`) and show it in the run header. Test in `server/tests/coverage.rs`.
  Note: new column `runs.narrowed_by text[]` (migration 0019) and `RunInfo.narrowed_by?: string[]` (absent when not narrowed). Narrowing uses the pinned revision's `inventory_hosts.groups`. A vars push naming no host the job reaches creates no run and audits `run.skipped` (reason `no_hosts`, context `narrowed_by`). Core gains `target::matching_paths`. Run header display is web (not done here).
- [X] T044 [P] [web] "covered by converge" in run host rows; `s068/covered.spec.ts`.
  Done: host rows link "by converge <id>", skipped rows give the reason, the header names
  `narrowed_by` (RunPage.tsx, run-068.ts).
- [ ] T045 (../ansible) Hand contracts/ansible.md §2–§3 (narrow jobs, covers, the paths
  recommendation) to the ansible session.

## Phase 7: User Story 5, check-mode runs (P2)

**Independent test**: `cargo test -p keel-server --test check_mode` (SC-004).

- [X] T046 [P] [core] `trust/ledger.rs` `Outcome::Checked`: tests that it does not move the base
  and that a replayed run id is still refused; implement.
- [X] T047 [P] [runner] Tests first, `runner/tests/check_mode.rs`: `check: true` adds
  `--check --diff`; signed mode writes `Checked`; base unchanged.
- [X] T048 [runner] Implement in `runner/src/executor/mod.rs` and `process.rs`.
- [X] T049 [P] [server] Tests first, `server/tests/check_mode.rs`: check run and check converge
  write no `host_revisions`, drift or rollback point; policy with `check: false` doesn't match a
  check run; listed with `check`.
  Note: three tests: a check run (`Assign.check` true, `RunInfo.check`, no `host_revisions`) then the same job for real (one revision); a check converge (every step a check run, every `Assign` checked, no revision); a priority-1 policy with `check: false` then `check: true`, upserted over the wire, flips which of a check or real run needs the catch-all approval. The server has no drift or rollback-point tables; `host_revisions` is the only write and is the one asserted.
- [X] T050 [server] `runs.check_mode` through `create_run`/`create_converge`, `Assign.check`,
  `finished()` skips revision writes; policy matching on `check`.
  Note: `create_converge` already stored `check_mode`. `NewRun.check` → `runs.check_mode`; `plan()` uses `policy::resolve_check`; `queue::assign` reads `check_mode` into `Assign.check`; `host_revision` returns early for a check run. `PreviewRunStart.check` no longer refuses `check_unsupported`, and its summary starts "Check (ansible-playbook --check --diff, changes nothing)". Protocol: `PolicySpec.check?` and `PolicyInfo.check?` (`Option<bool>`, optional) so policies can be set and listed on `check`. A check converge covers nothing (US4 coverage skips `check_mode` converges).
- [X] T051 [P] [web] Check toggles (Start dialog, converge preview), `check` badges in every run
  list; `s068/check-mode.spec.ts`.
  Done: new StartRunDialog.tsx (Repository > Jobs > Run) with the Check switch; CheckBadge and
  RunChecks in checks.tsx on run page and converge page. Check on policies deferred: the web app
  has no policies UI yet.

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
- [X] T056 [P] [web] `env_value` kind in the credentials UI; credential names shown in previews.
  Done: "Environment value" kind with a hint (BindingsPage.tsx); the Start dialog lists the
  credentials sent; `s068/env-credential.spec.ts`.

## Phase 9: User Story 7, runnable only once checks passed (P3)

**Independent test**: `cargo test -p keel-server --test required_checks`.

- [X] T057 [P] [core] `Forge::commit_statuses` in `core/ports.rs`; Forgejo and GitHub adapters in
  `forge/` with recorded-fixture tests (`cargo test -p keel-forge statuses`); `Unsupported` for
  others.
  Note: signature is `commit_statuses(sha)` (adapters are already repo-bound, `RepoRef` lives in keel-forge); core state enum is `CommitStatusState` (no `Missing`: the server derives it); GitHub reads one page of 100 check runs.
- [X] T058 [P] [server] Tests first, `server/tests/required_checks.rs`: `waiting_checks →
  queued` on all success; `checks_failed`; `checks_timeout`; preview refused on an unsupported
  forge; converges and runs both gated.
  Note: six tests against a fake forge whose commit statuses the test sets: refusals (`checks_unsupported` with no forge connection and on a forge without statuses, `invalid_context`, `too_many_contexts`, `invalid_timeout`, `no_change`); a run waits with `RunInfo.checks` showing pending/missing and nothing assigned, then queues and succeeds once all pass (`run.checks_passed` audited); checks come before approval (the approval window starts when they pass); a failure ends it `checks_failed` with hosts cancelled and the failing context audited; backdating past the timeout ends it `checks_timeout`; a converge waits too with every step blocked; a waiting run can be cancelled.
- [X] T059 [server] `server/src/checks.rs`: required-check commands (step-up), `waiting_checks`
  at create, poller on the sync cadence, `run_check_states`, `RunChecksChanged`, span
  `checks.wait`.
  Note: Deviations: the generic `Confirm` replaces the contract's `ConfirmRequiredChecks`; `PreviewRequiredChecks` also takes an optional `timeout_secs` (60..=86400). New `Event::RunChecksChanged { run_id, checks }` and `RunInfo.checks?`. The gate sits before review/approval/queue for both manual runs and converges (converge approval no longer goes straight to queued while checks are required); the post-checks state is derived from `review_groups`/`approvals_needed`, so no migration. Contexts are snapshotted on the run when it is made. Polling is its own 30 s loop (`checks::spawn_poller`), reading statuses once per (binding, commit); a forge read failure keeps the run waiting until its timeout. keel-core gained `waiting_checks -> cancelled` for user cancel and binding disable.
- [X] T060 [P] [web] Binding Required checks section; `waiting_checks` display with forge links;
  `s068/required-checks.spec.ts`.
  Done: bindings/RequiredChecks.tsx (add, remove, timeout via PreviewRequiredChecks + Confirm,
  inline refusals incl. checks_unsupported); live RunChecksChanged in runs/store.ts. Fakes in
  tests/support/fake-runs-068.ts.

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
- [X] T064 [P] [server] Tests first, `server/tests/choice_params.rs`: allowed values at the
  pinned commit, refusal of other values, `PreviewRunStart` with params and check.
  Note: three tests against a `provision` job pushed onto the converge fixture (`stack: {kind: choice, from: composes/, default: web}`, `size: {kind: choice, values: [small, large]}`): `ListJobs` shows allowed values at the binding head (hidden dirs dropped, a newly pushed dir appears after sync); out-of-list, hidden, ill-shaped and undeclared values are refused `param_invalid` with no run made and the value never echoed; an accepted check start carries params and `check` into `RunInfo` and `Assign`.
- [X] T065 [server] `RepoTree::list_dirs` from the mirror/`repo_catalog`; `ParamView.allowed`;
  `PreviewRunStart.params`/`check` through `check_params` (`runs.rs:177`).
  Note: `RepoTree::list_dirs` was already implemented by the mirror tree (`TreeAt`); the server reads it through a new `with_tree` (mirror in `spawn_blocking`, an empty tree when the mirror or commit is missing, so `from:` fails closed). There was no `ParamView` on the wire: `JobInfo.params?: Record<string, ParamView>` is new, with `ParamView { kind, allowed, default, unavailable }` resolved at the binding head (the commit a start pins). `PreviewRunStart.params` is checked by the new `check_params_at` against the pinned commit and refused `param_invalid` before the preview; the `params_unsupported` refusal is gone. The summary lists params (" Params: k=v, …."). Converge step params are untouched.
- [X] T066 [P] [web] Start dialog params with a `choice` select; `s068/choice-param.spec.ts`.
  Done: choice Select (disabled with its reason when unavailable), oci_digest input,
  param_invalid shown inline.

## Phase 11: Polish and cross-cutting

- [ ] T067 [P] `docs/guides/keel/converge.md` (recipes, converge, profiles and suggestions,
  covers, check mode, env credentials, required checks, choice params, the runner-upgrade order),
  and a CONTRIBUTING note on the `converge_advance` trigger.
- [ ] T068 (../ansible) Hand the whole contracts/ansible.md to the ansible session in the order
  of quickstart §4 (runners upgraded, ceilings, then manifest); record its confirmation of SC-002.
  Handed over 2026-11-18. ../ansible: §5 ceilings + debian on main (9a61c06); branch
  `keel-068-manifest` = 7a17a57 (keel-manifest-check knows 068 keys and covers rules), 4fe40ff
  (provision `-e stack=` wins), 90af010 (§1–§4, §6, §7: held until "0 runners would refuse").
  `keel_runner_queries` already exists (48cb22d, absent = refuse all). Open: the 068
  server+runner release with SHA256SUMS (blocks the sha bump), then SC-002 once commit 3 lands.
- [ ] T074 A `choice` (or any) param with no `default` is required at dispatch: today
  `check_params` (apps/keel/server/src/runs.rs) only validates params that were sent, so an unset
  `stack` reaches provision.yaml, whose own guard is the only refusal. Refuse `param_invalid`
  ("param stack is required") at StartRun and in the Start dialog.
- [X] T069 [web] Impeccable pass over every 068 screen together (Hosts, host page, converge
  preview and page, Start dialog, binding page, enrollment). Done with 067's Queries panel:
  Hosts columns no longer overlap at 1280 (fixed State/Converge widths, kit DataTable
  `actionsWidth`, Converge cell wraps and shows "—" when empty); host and binding panel
  sections take H3 with wider gaps between them; Last converge link carries its state tone;
  Queries controls share one height and show a loading state; Require/Save/Grant match field
  height; dialog Cancel is ghost everywhere and Start run uses default-size footer buttons.
- [X] T074 [core][server][web] A `choice` param with no default is required, and its job is
  manual-only. Done: core refuses one with `paths:` (`required_param_paths`) or in a recipe
  (`manual_only_recipe_job`), and a converge skips it as `HostPlan.skipped` (`needs_value`).
  The server refuses a Start without it ("The run's param <name> is required."), and push never
  runs it. The Start dialog marks it required and keeps Review run off until a value is picked.
- [ ] T070 Main session only: `scripts/hygiene.sh`, then the full gate once on the combined
  branch (`scripts/ci.sh`); report wall time, slowest steps and delta vs the last pass.
- [ ] T071 [web] Brochure screens last: add the converge preview, converge page and Profiles
  card to `web/tests/e2e/polish-screens.spec.ts` and regenerate.

## Dependencies & execution order

- Phase 1 → Phase 2. In Phase 2, T005–T013 ([core]) block T014–T015 only where types are used;
  T014 can start with T005.
- US1 (Phase 3) needs Phase 2. US2, US3 and US4 need US1's server tasks T018–T022. US5–US8 need
  only Phase 2 and can run in parallel with US1.
- T045/T068 wait for runners on 068 (research R4) and the ansible delta adding `debian` to the ceilings (Q1, decided 2026-11-17).
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

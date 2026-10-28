---
description: "Task list for 058: onboard the Ansible repo to Keel"
---

# Tasks: Onboard the Ansible repo to Keel

**Input**: `specs/058-keel-onboard-ansible/` (plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md)

**Tests**: Included. Constitution III is non-negotiable, and each contract names its tests.

**Paths**: Plain paths are in fathom. Paths starting `../ansible/` are on the `../ansible`
integration branch `keel-058` (T001).

**Rules for every task**:
- Subagents never run `scripts/ci.sh`, `paws` or `dagger`. They use targeted tests only:
  `cargo test -p <crate>`, `make test ROLE=<role>` in `../ansible`, or one Playwright spec.
  One command at a time, with `CARGO_BUILD_JOBS=4`.
- Commit each task signed. Do not push.
- Never log a token, a key or an age secret. Use `no_log` on token tasks. No IPs in git.

## Format: `[ID] [P?] [Story] Description`

---

## Phase 1: Setup

- [X] T001 Create the integration branch `keel-058` in `../ansible` from `main`, then cherry-pick the 7 commits of `keel-runner-trial` onto it (`mcp__gitops__cherry_pick`, signed, attribution kept). Record the branch in `../ansible/CONTRIBUTING.md` § Keel (new section, stub).
- [X] T002 Constitution PATCH 1.4.0 → 1.4.1 in `.specify/memory/constitution.md` (research R13). The Keel host runner tier wording "jobs run as the automation account" becomes "jobs run as a dedicated executor account that no human shares". Bump the version, set Last Amended, and add a Sync Impact comment, which is removed before commit.

---

## Phase 2: Foundational (amendment A1, blocks US1 and US2)

See contracts/keel-amendments.md § A1.

- [X] T003 [P] Rename the executor account `automation` → `keel-exec`, and the sudoers file `/etc/sudoers.d/keel-automation` → `/etc/sudoers.d/keel-exec`, throughout `specs/055-ansible-repo-control-plane/` (`contracts/runner.md`, `contracts/host-install.md`, `plan.md`, `quickstart.md`, `research.md`, `spec.md`).
  - Add a "Prerequisite check" section to `contracts/host-install.md`, from research R3.
  - Add a "Trial migration" section to the same file, from research R4.
- [X] T004 [P] Write a failing test in `apps/keel/runner/tests/units.rs` that parses `apps/keel/runner/systemd/keel-executor@.service` and `keel-runner.service`. It asserts `User=keel-exec`, `Group=keel-exec`, and `RestartPreventExitStatus=78` on `keel-runner.service`.
- [X] T005 Make T004 pass in `apps/keel/runner/systemd/keel-executor@.service` and `apps/keel/runner/systemd/keel-runner.service`.
- [X] T006 Replace every literal executor account name and doc comment `automation` with `keel-exec` in `apps/keel/runner/src/bin/keel-executor.rs`, `apps/keel/runner/src/lib.rs` and `apps/keel/runner/src/executor/`. Gate: `cargo test -p keel-runner`.
- [ ] T007 Re-vendor both unit files into `../ansible/roles/keel_runner/files/systemd/`, byte-identical to fathom's.

**Checkpoint**: Keel's runner and its 055 docs name `keel-exec`, and the constitution is at 1.4.1.

---

## Phase 3: User Story 1 — close the root paths; the executor gets its own account (P1) 🎯 MVP

**Goal**: No path from the `automation` group to root. The executor's root grant exists only on a
host where that is verified.

**Independent test**: `make test ROLE=base` and `make test ROLE=keel_runner` pass every case in
contracts/base-hardening.md § Tests and keel-amendments.md § A1 Tests.

### Tests first

- [ ] T008 [P] [US1] Extend `../ansible/roles/base/molecule/default/tests/test_default.py` with the following failing cases.
  - A group member cannot write `/root/ansible/.git/hooks/x`, `.git/config` or `playbooks/x.yaml`.
  - `sudo automation-compose-up` exits as follows:
    - 64 with no arguments, with two arguments, with `'x up -d -f /tmp/y'`, and with `../x`;
    - 77 for an unlisted name;
    - 66 for a listed name with no compose file.
  - A listed name with a compose file runs.
  - `sudo -l -U <member>` shows no `docker` rule.
  - Converge is idempotent (a second converge reports `changed=0`).
- [ ] T009 [P] [US1] Create Molecule scenarios for `../ansible/roles/keel_runner/molecule/`, each with testinfra tests.
  - `default`: fresh install. It creates the `keel-exec` user and group, with nologin and home `/var/lib/keel-executor` `0700`. `/etc/sudoers.d/keel-exec` contains exactly `keel-exec ALL=(root) NOPASSWD: ALL` and `Defaults:keel-exec !requiretty`, and passes `visudo -cf`.
  - `migrate`: a trial `automation` user with home `/var/lib/keel-executor` and a nologin shell is converted. Ledger and Git store bytes are unchanged (compare sha256), the `automation` group still exists, and `/etc/sudoers.d/keel-automation` is gone.
  - `refuse`: the role fails with no `keel-exec` sudoers file in three cases:
    - a human `automation` user exists (a login shell, or any other home);
    - `/etc/sudoers.d/automation` contains `docker compose`;
    - `getfacl /root/ansible` shows group `w`.

### Implementation

- [ ] T010 [US1] In `../ansible/roles/base/tasks/main.yml`, set the group ACL on `/root/ansible` to `r-X`, recursive, for both the access and default ACL, with any group `w` removed. Skip when the path is absent, and make it idempotent (research R1).
- [ ] T011 [US1] Add `../ansible/roles/base/templates/automation-compose-up.sh.j2`, installed as `/usr/local/sbin/automation-compose-up` (root, `0755`). It is POSIX sh, with no sourcing and no `eval`, and uses the exact exit codes and `exec env -i PATH=… docker compose -f /root/ansible/composes/NAME/compose.yaml up -d` from contracts/base-hardening.md.
- [ ] T012 [US1] In `../ansible/roles/base/tasks/main.yml` and `../ansible/roles/base/defaults/main.yml`, add `/etc/automation/composes.list` (root:root `0644`), built from `base_automation_composes`. Its default is `[inventory_hostname]` when `composes/<inventory_hostname>/compose.yaml` exists, else `[]`.
- [ ] T013 [US1] Replace the free-form `base_automation_sudo_commands` with a template `../ansible/roles/base/templates/sudoers-automation.j2`.
  - It holds the exact `AUTOMATION_GIT` and `AUTOMATION_COMPOSE` aliases from contracts/base-hardening.md.
  - `AUTOMATION_GIT` is gated by `base_automation_git_rules` (default `true`).
  - Mode `0440`, validated by `visudo -cf %s`.
  - Remove the old variable from defaults; fail with a clear message if an inventory still sets it.
- [ ] T014 [US1] In `../ansible/roles/keel_runner/tasks/account.yml` (new), included from `main.yml`, create the `keel-exec` user and group (research R3):
  - group `keel-exec`, plus membership of `keel-trust`;
  - nologin shell;
  - home `/var/lib/keel-executor`, mode `0700`.
  - Remove the role's creation of the `automation` user and group.
- [ ] T015 [US1] In `../ansible/roles/keel_runner/tasks/migrate.yml` (new), run before `account.yml` (research R4).
  - Detect the trial user: name `automation`, home `/var/lib/keel-executor`, nologin shell.
  - Then, in order:
    1. stop `keel-runner.service` and `keel-executor.socket`;
    2. create `keel-exec`;
    3. chown `/var/lib/keel-executor` recursively;
    4. `userdel` `automation`, without `-r` and without removing the group;
    5. remove `/etc/sudoers.d/keel-automation`;
    6. restart the units.
  - Fail closed if an `automation` user exists that doesn't match the trial shape.
- [ ] T016 [US1] In `../ansible/roles/keel_runner/tasks/grant.yml` (new), assert on live state:
  - `/etc/sudoers.d/automation` has no `docker compose` and does have `/usr/local/sbin/automation-compose-up`;
  - `getfacl -R /root/ansible` (when present) shows no group `w`;
  - no human account is in group `keel-exec`.
  - Only then write `/etc/sudoers.d/keel-exec` (visudo-validated, `0440`).
  - If any assertion fails, remove an existing grant and fail.
- [ ] T017 [US1] Point `../ansible/roles/keel_runner/files/systemd/` and the role's tasks and templates at `keel-exec`. Ensure `playbooks/keel-node.yaml` runs `base` hardening first, or asserts on it, through T016. Gate: `make test ROLE=base` and then `make test ROLE=keel_runner`.

**Checkpoint**: US1 independent test passes.

---

## Phase 4: User Story 2 — one command onboards a host (P1)

**Goal**: `KEEL_TOKEN=<enroll or join token> make keel-node` takes a fresh host to enrolled.

**Independent test**: quickstart.md § 2 on a throwaway VM.

- [ ] T018 [P] [US2] In `../ansible/Makefile`, `keel-node` runs `playbooks/debian.yaml` and then `playbooks/keel-node.yaml` through `KEEL_PLAYBOOK`, in one invocation (research R5). `make debian keel-node` still works. Update the help text and the examples line to `KEEL_TOKEN=<token> make keel-node`.
- [ ] T019 [P] [US2] In `../ansible/roles/keel_runner/tasks/main.yml`, fail when `inventory_hostname` is not a host in `inventory/hosts.yml` and Keel is in use. The message names the hostname and the file (research R6, FR-013).
- [ ] T020 [US2] In `../ansible/roles/keel_runner/tasks/token.yml`:
  - accept an enroll or a join token from `KEEL_TOKEN` only, and keep `no_log: true` on every task that touches it;
  - for a join token, print the host key fingerprint (`ssh-keygen -lf` of the runner's host key) for the owner to match in Keel (FR-007);
  - after enrolment, assert that no token remains on disk (FR-006).
- [ ] T021 [US2] Move the SOPS age key (research R7), in `../ansible/roles/sops/tasks/main.yml` and `../ansible/roles/sops/defaults/main.yml`:
  - new path `/etc/sops/age/keys.txt` (`sops_age_key_file`);
  - directory `0750` and file `0440` root:keel-exec when the `keel-exec` group exists, else root `0400`;
  - a symlink from `/root/.config/sops/age/keys.txt`.
  - Add `age_keyfile: "{{ sops_age_key_file }}"` to `load_vars` in `../ansible/roles/k3s_backup/tasks/main.yml` and `../ansible/roles/keel_server/tasks/main.yml`.
  - Gate: `make test ROLE=sops`.
- [ ] T022 [US2] Replace the `REPLACE_WITH_SHA256` placeholders in `../ansible/inventory/group_vars/keel_nodes.yml` with the real checksums of a runner release built from Phase 2. The release build and upload are the owner's step; record the version in `keel_runner_release` (FR-008).

**Checkpoint**: quickstart.md § 2 passes for an enroll token, a join token and a trial-migrated VM.

---

## Phase 5: User Story 3 — profiles and jobs in the repository (P1)

**Goal**: `.keel/jobs.yaml` declares the eight profiles and ten jobs from contracts/jobs-manifest.md, and a check keeps the paths honest.

**Independent test**: `make lint` passes. A change under `roles/base/` queues only the jobs that reach `base`. A change to `inventory/hosts.yml` queues nothing.

- [X] T023 [P] [US3] Write failing tests in `../ansible/scripts/tests/test_keel_manifest_check.py` (pytest, fixtures under `scripts/tests/fixtures/`). Each case must fail the check:
  - a missing role;
  - a role reached through `include_role` that the job's paths don't list;
  - an `import_playbook` that isn't followed;
  - a path that matches no file;
  - an undeclared profile.
- [X] T024 [US3] Implement `../ansible/scripts/keel-manifest-check.py`, in pure Python 3 with no Ansible import. It follows `roles:`, `import_playbook`, and literal `include_role`/`import_role` recursively (contracts/jobs-manifest.md § Check), and must finish in under 5 s on the repo.
- [X] T025 [US3] Write `../ansible/.keel/jobs.yaml` exactly as contracts/jobs-manifest.md describes.
  - Profiles: debian, prox-vm, gameserver, host, k3s-master, k3s-agent, keel-server and keel-node.
  - Jobs, with their paths: one per profile, plus manual-only `upgrade` and `clean`, which have no `paths` and cover all eight profiles.
  - Shared paths: `ansible.cfg`, `requirements.yml` and `inventory/group_vars/all.yml`.
  - Timeouts as listed. Keel's 055 manifest parser must accept the file; check it with a fathom unit test, or with `keel` if it has a validate command.
- [X] T026 [P] [US3] In `../ansible/roles/tailscale/tasks/main.yml`, make the role a no-op when the host is already on the tailnet and `TS_AUTHKEY` is unset (contracts/jobs-manifest.md § Conditions). Add a Molecule case for it if the role has a scenario.
- [X] T027 [P] [US3] Audit every job playbook in `../ansible/playbooks/` for `vars_prompt`, `pause` and required run-time variables, and remove them or give them defaults so each job runs unattended.
- [ ] T028 [P] [US3] In `../ansible/roles/keel_server/tasks/main.yml`, when `KEEL_EVENT_FD` is set, defer `docker compose up -d` with a systemd transient timer 30 s after the job, in the same way `keel_runner` defers its restart. Without it, run `docker compose up -d` at once (research R9).
- [X] T029 [US3] Wire `scripts/keel-manifest-check.py` and its tests into `../ansible/Makefile` `lint`.

**Checkpoint**: US3 independent test passes on the integration branch.

---

## Phase 6: User Story 4 — trust pinned in the repository, signed mode on (P1)

**Goal**: The trust set lives in group vars as contracts/trust-vars.md describes, and is checked.

**Independent test**: quickstart.md § 4.

- [X] T030 [P] [US4] Write failing tests in `../ansible/scripts/tests/test_keel_trust_check.py`. Each case must fail the check:
  - `keel_nodes.yml` with `allowed_profiles`;
  - `require_signed: false`;
  - no signer or no approver;
  - a `REPLACE_WITH` placeholder;
  - a `keel_nodes` host with allowed profiles from zero or two functional groups;
  - an IP literal under `inventory/`.
- [X] T031 [US4] Implement `../ansible/scripts/keel-trust-check.py` and wire it into `make lint` in `../ansible/Makefile`.
- [X] T032 [US4] Edit `../ansible/inventory/group_vars/keel_nodes.yml`:
  - set `keel_runner_require_signed: true`;
  - remove `keel_runner_allowed_profiles`;
  - leave `keel_runner_trusted_signers` and `keel_runner_approvers` as documented, empty lists that the trust check reports until the owner fills them (public keys and COSE entries only).
- [X] T033 [P] [US4] Add `keel_runner_allowed_profiles` to `../ansible/inventory/group_vars/compose_hosts.yml`, `pve_hosts.yml`, `k3s_servers.yml`, `k3s_agents.yml` and `keel_servers.yml`, using the values from contracts/trust-vars.md. Gameserver hosts get `[gameserver, keel-node]` through a `gameservers` group in `inventory/hosts.yml` with `inventory/group_vars/gameservers.yml`.
- [X] T034 [P] [US4] Document the trial override (`require_signed: false`, `allowed_profiles: [debian, keel-node]`) in `../ansible/inventory/host_vars/README.md`.
- [ ] T035 [US4] Owner step, recorded only: add your signing key to `keel_runner_trusted_signers` and your passkey entry (from Keel's passkey page) to `keel_runner_approvers`. `make lint` then passes the trust check. The commit is signed.

**Checkpoint**: The trust check passes, and an unsigned commit is refused on a signed-mode VM.

---

## Phase 7: User Story 6 (part 1) — break-glass shows in Keel (amendment A2) (P2)

Comes before US5 because R12's per-host gate needs it. See contracts/keel-amendments.md § A2 and data-model.md.

**Independent test**: quickstart.md § 5.

### Tests first

- [ ] T036 [P] [US6] In `crates/keel-exec/tests/frames.rs`, add round-trip tests for:
  - `OutsideRuns { entries: Vec<OutsideRun>, skipped: u32 }`, with at most 100 entries per frame;
  - `OutsideRunsAck { through: u64 }`.
  - A frame with more than 100 entries is refused.
- [ ] T037 [P] [US6] Add a runner unit test, in `apps/keel/runner/src/runner/outside.rs` (`#[cfg(test)]`), for:
  - reading from the saved offset;
  - advancing only on ack;
  - reading from 0 after truncation;
  - skipping lines over 1 KiB, malformed lines, `user` over 64 bytes and `playbook` over 256 bytes, and counting each in `skipped`.
- [ ] T038 [P] [US6] Add server tests in `apps/keel/server/tests/outside_runs.rs` for:
  - insert-only storage;
  - de-duplication on `(host_id, at, playbook, coalesce(commit,''))`;
  - `check: false` sets `changed_outside`, and `check: true` does not;
  - a baseline approval clears the flag;
  - completing every assigned job after the newest entry clears the flag;
  - each received batch is audited.
- [ ] T039 [P] [US6] Write the Playwright spec `apps/keel/web/tests/e2e/us10/changed-outside.spec.ts`. The **Changed outside Keel** badge appears live on the host row and in the host view; entries are labelled as the host's claims; the `changed_outside` filter works; a baseline approval clears the badge.

### Implementation

- [ ] T040 [US6] Add `OutsideRun`, `OutsideRuns` and `OutsideRunsAck` to `crates/keel-exec/src/frames.rs`, on the runner subsystem, with the direction rules. Gate: `cargo test -p keel-exec`.
- [ ] T041 [US6] Add the migration `apps/keel/server/migrations/0009_outside_runs.sql`.
  - Table `host_outside_runs`, insert-only (enforced by a trigger, as for existing insert-only tables), with these columns:
    - `id uuid PK`, `host_id uuid FK hosts`, `at timestamptz`, `received_at timestamptz`;
    - `user text CHECK (octet_length(user) <= 64)`;
    - `playbook text CHECK (octet_length(playbook) <= 256)`;
    - `commit text NULL CHECK (commit ~ '^[0-9a-f]{40}$')`;
    - `dirty bool`, `check bool`.
  - A unique index on `(host_id, at, playbook, coalesce(commit,''))`.
  - `ALTER TABLE hosts ADD changed_outside bool NOT NULL DEFAULT false`.
- [ ] T042 [US6] In `apps/keel/runner/src/runner/outside.rs` (new) and `apps/keel/runner/src/runner/session.rs`:
  - read `/var/lib/keel-outside/runs.jsonl` on `Hello` and every 60 s while connected;
  - persist the offset in the runner's state directory, advancing it only on `OutsideRunsAck`.
- [ ] T043 [US6] Ingest outside runs in `apps/keel/server/src/runner_listener.rs` and `apps/keel/server/src/hosts.rs`:
  - insert with de-duplication, audit the batch (`apps/keel/server/src/audit.rs`), set the flag, and emit `HostChangedOutside`;
  - clear the flag on baseline approval (`apps/keel/server/src/governance.rs` or wherever baselines are approved) and on job completion (`apps/keel/server/src/runs.rs`).
- [ ] T044 [US6] In `crates/keel-protocol/src/host.rs` and `crates/keel-protocol/src/event.rs`:
  - `GetHost` gains `changed_outside` and the newest 20 entries;
  - `ListHosts` gains a `changed_outside` filter;
  - add the event `HostChangedOutside { host }`.
  - Add round-trip tests and regenerate `apps/keel/web/src/protocol/generated/`. Gate: `cargo test -p keel-protocol`.
- [ ] T045 [US6] Add the badge, filter and entry list to `apps/keel/web/src/features/hosts/HostsPage.tsx` and `apps/keel/web/src/features/hosts/store.ts`, live from the event. Gate: the Vitest unit tests, then `pnpm --filter @companion/keel-web exec playwright test tests/e2e/us10`.
- [ ] T046 [P] [US6] Write the callback `../ansible/callback_plugins/keel_outside.py`.
  - When `KEEL_EVENT_FD` is unset and `/var/lib/keel-outside/` exists, it appends one JSON line `{"v":1,"at","user","playbook","commit","dirty","check"}` to `runs.jsonl`.
  - Enforce the field bounds before writing.
  - On error, print one warning and never fail the run.
  - Enable the callback in `../ansible/ansible.cfg` (`callbacks_enabled`, and `callback_plugins = callback_plugins`).
  - Add pytest coverage in `../ansible/scripts/tests/test_keel_outside.py`.
- [ ] T047 [US6] In `../ansible/roles/keel_runner/tasks/main.yml`, create `/var/lib/keel-outside/` (root, `0755`) and `runs.jsonl` (root, `0644`), and add the paths to the Molecule `default` verify.
- [ ] T048 [US6] Update 055's `contracts/runner.md` (frames), `data-model.md` (table and flag) and `quickstart.md` in `specs/055-ansible-repo-control-plane/` to match A2.

**Checkpoint**: quickstart.md § 5 passes on the trial VM.

---

## Phase 8: User Story 5 — enrol existing hosts one at a time (P2)

**Goal**: Every host is onboarded in R12's order, each passing the four gates before the next starts.

**Independent test**: quickstart.md § 6 for each host.

- [ ] T049 [US5] Add `../ansible/docs/keel-onboarding.md`, a log with one row per host in R12's order:
  - trial VM;
  - `audiobookshelf` or `pulse`;
  - `rustfs`, `steam`;
  - Keel server host;
  - `forgejo`, `keycloak`, `pve`;
  - k3s agent, k3s server.
  - Columns: enrolled, baseline commit, first-run result, outside-run seen, date.
- [ ] T050 [US5] Make every host to onboard appear in `../ansible/inventory/hosts.yml` under its hostname and in exactly one functional group. Keep addresses in ignored host_vars (FR-009, FR-013). Gate: `make lint`.
- [ ] T051 [US5] Owner step: onboard the trial VM and the first compose host. Prove break-glass on the compose host: stop Keel, run `make prox-vm`, start Keel, and see the badge (US6 scenario 1–2). Record the result in the log.
- [ ] T052 [US5] Owner step: onboard `rustfs` and `steam`, then the Keel server host, then `forgejo` and `keycloak`. For each, also prove that host's own break-glass (US6 scenario 3 for `keycloak`). Record the results in the log.
- [ ] T053 [US5] Owner step: onboard `pve`, then the k3s agent, then the k3s server. For the off-LAN k3s server, confirm it can reach the runner port first (edge case). Record the results in the log.

---

## Phase 9: User Story 6 (part 2) — retire the manual pull path (P2)

**Independent test**: On every host, `sudo -l -U <member>` shows no `git` rule.

- [ ] T054 [US6] Once T053 is complete (SC-002), set `base_automation_git_rules: false` in `../ansible/inventory/group_vars/all.yml`. Let Keel deliver the change to every host. Confirm on each host with `sudo -l` and record it in `../ansible/docs/keel-onboarding.md`.

---

## Phase 10: Polish and cross-cutting

- [ ] T055 [P] Write `../ansible/CONTRIBUTING.md` § Keel (FR-020). It covers:
  - onboarding a host;
  - accepting a join request (fingerprint match);
  - upgrading runners (bump the release and checksums);
  - break-glass, including for the Keel server, `keycloak` and `forgejo` hosts;
  - the limits of the outside-run record;
  - rotating signers, approvers and the join token;
  - the trial host_vars.
- [ ] T056 [P] Update the `README.md` in `../ansible` with one line pointing at Keel and at CONTRIBUTING.
- [ ] T057 Run `make lint` and `make test` in `../ansible` (one role at a time if the machine is loaded).
- [ ] T058 Main session only: run the full fathom gate `scripts/ci.sh` once, on the combined branch.
- [ ] T059 Run `scripts/hygiene.sh`. Delete any Molecule containers and kind clusters the tasks created, and prune the build cache with `docker builder prune --filter until=24h`.
- [ ] T060 Update the checklist status in `specs/058-keel-onboard-ansible/spec.md` to Implemented, and tick these tasks.

---

## Dependencies and execution order

```text
Phase 1 (T001, T002)
  └─▶ Phase 2: A1 (T003–T007)
        ├─▶ US1 (T008–T017) ─▶ US2 (T018–T022) ─┐
        ├─▶ US3 (T023–T029) ────────────────────┼─▶ US6 part 1: A2 (T036–T048) ─▶ US5 (T049–T053) ─▶ US6 part 2 (T054)
        └─▶ US4 (T030–T035) ────────────────────┘
Polish (T055–T060) after T048; T057–T060 last.
```

- US2 needs US1, because the root grant must be gated before any real host gets it.
- US3 and US4 touch different files from US1 and US2, so they can run alongside them on the same branch.
- A2's fathom half (T036–T045, T048) depends only on Phase 2. Its `../ansible` half (T046–T047) needs US1.
- US5 needs US1–US4 and A2, because each host's gate includes "outside run seen".

## Parallel opportunities

- **Phase 2**: T003 ∥ T004.
- **US1**: T008 ∥ T009, then T010–T013 (base) ∥ T014–T016 (keel_runner).
- **US3**: T023 ∥ T026 ∥ T027 ∥ T028.
- **US4**: T030 ∥ T033 ∥ T034.
- **A2**: T036 ∥ T037 ∥ T038 ∥ T039 ∥ T046. After those:
  - T040 → T042;
  - T041 → T043;
  - T044 → T045.
- **Fan-out by repo**: one agent on fathom's A2 (T036–T045), one on `../ansible` base and keel_runner (US1), one on `../ansible` manifest and trust (US3/US4). Each works in its own worktree, and only the main session merges.

## Implementation strategy

- **MVP**: Phases 1–3 (A1 plus US1). The root paths are closed and the grant is gated. Only then
  is it safe to run `make keel-node` on any real host.
- **Next**:
  - US2–US4 make the repo Keel-ready;
  - A2 makes break-glass visible;
  - US5 rolls out host by host;
  - US6 part 2 cleans up.
- Owner steps (T022 release, T035 keys, T051–T053 onboarding) are not agent work. Agents prepare
  them and stop.

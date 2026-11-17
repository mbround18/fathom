# Feature Specification: Keel host provisioning and convergence

**Feature Branch**: `068-keel-converge`

**Created**: 2026-11-17

**Status**: Draft

**Input**: Owner, relayed by the infrastructure session on 2026-11-17: "observe all the patterns we
do here in this ansible repo, we should support them in keel". The scope is the four gaps found in
Keel's job-to-host mapping:

1. editing an enrolled host's profiles;
2. a converge trigger on enrollment and on demand;
3. ordering between jobs, with a failure policy;
4. deriving profiles from inventory groups.

It also takes in the operator repository patterns listed under "Why".

Decisions taken with the owner (2026-11-17):

- **Order comes from profile recipes** in `.keel/jobs.yaml`. A profile names an ordered list of
  jobs and may include other profiles first. There is no general job dependency graph.
- **Failure stops that host, and other hosts go on.** A failed step cancels the rest of that
  host's chain (`skipped: dependency failed`). A retry resumes from the failed step.
- **One approval covers the plan.** The approver sees every host × step, the commit and the
  params, and approves once. Each step still runs under signed-mode review.
- **Enrollment converges automatically**, through the approval policy. A checkbox at enrollment
  turns it off for that host.
- **Groups suggest profiles; an admin confirms.** `jobs.yaml` maps inventory groups to profiles.
  Keel proposes the result and never assigns a profile on its own.
- **A converge absorbs an overlapping narrow job.** A path-triggered job is folded into a
  converge queued or running on the same host when that converge covers it at the same or a newer
  commit. Otherwise the two queue one after the other under the per-host lock.
- **In scope**: check-mode runs, environment-variable credentials, a lint gate before a revision
  becomes runnable, and manual-only one-off jobs.
- **Secrets**: environment-variable credentials resolve through Keel's existing `SecretResolver`
  port (`crates/keel-core/src/ports.rs:97`). Today that is Keel's sealed credentials. Spec 059 adds
  the suite secret manager as a second adapter (059 FR-009), and moving SOPS-held values into it is
  059's work. SOPS files stay where they are: decrypted on the host with its own age key, with 066
  checking recipients.

**Builds on**:

- spec 055 (jobs, profiles, runs, approvals, signed mode, enrollment, credentials);
- spec 061 (job params);
- spec 064 (run approval);
- spec 066 (repository catalog, sops awareness, Add host);
- spec 067 Phase 0: D2, one run per host, and D3, join tokens. This branch starts after D2.

## Why

The operator's Ansible repository (`../ansible`) is driven by `make` today. Keel runs one
playbook per job against the host it was sent to. Comparing the two (2026-11-17, with file
references in research.md) shows what Keel cannot yet express:

- **Profiles are chains.** `make keel-node` runs `debian.yaml` then `keel-node.yaml`, and `make
  keel-relay` runs `debian.yaml` then `keel-relay.yaml`. Keel's `keel-node` job runs only the
  second playbook, so it assumes the baseline already ran. A fresh box needs the whole chain, in
  order.
- **Narrow jobs live inside wide ones.** `k3s-audit`, `k3s-addons`, `k3s-io-priority`,
  `k3s-backup` and `tailscale` are standalone playbooks. Their roles are also part of
  `k3s-master.yaml` and `k3s-agent.yaml`. A change to `roles/k3s_audit` should run the narrow job,
  not the whole node playbook, and the narrow job must not run beside a converge that already
  applies it.
- **Groups decide profiles.** Every playbook targets inventory groups (`k3s_servers`,
  `k3s_agents`, `keel_servers`, `keel_nodes`, `compose_hosts`, `pve_hosts`, `gameservers`). The
  same groups set each host's local profile ceiling (`keel_runner_allowed_profiles` in
  `group_vars`). In Keel an admin assigns profiles by hand at enrollment and cannot change them
  afterwards.
- **Onboarding is self-enrolling.** `keel-node.yaml` and `keel-server.yaml` use `group_by` to
  put the target into their group for the run, and refuse a multi-host run of hosts not yet
  listed. A new box needs no inventory edit first.
- **Every target has a dry run.** `CHECK=1` adds `--check --diff`. Keel's executor has none.
- **Secrets come from the environment.** `TS_AUTHKEY`, `K3S_URL` and `K3S_TOKEN` are read with
  `lookup('env')`, and SOPS files are decrypted on the box. Keel has `env` credentials, but no
  job in `.keel/jobs.yaml` declares any.
- **Rollback is per playbook.** `keel-server` keeps `digest.previous`, and the k3s roles snapshot
  etcd and roll back in a `rescue` block. Keel's `keel-server` job already takes the digest as a
  param (`keel_server_image_digest`, spec 061). No job declares `rollback`.
- **Some targets are one-offs.** `upgrade` and `clean` are already manual-only Keel jobs.
  `provision` prompts for a compose stack. `template` builds a Proxmox VM template from a shell
  script on the hypervisor.
- **The repository checks itself.** `make lint` runs the manifest and trust checks, ansible-lint
  and a syntax check of every playbook. `make test` runs molecule. Keel binds a revision without
  knowing whether those passed.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Converge a host to its profiles (Priority: P1)

As an operator, I can ask Keel to bring a host fully up to its assigned profiles. Keel expands
each profile's recipe into an ordered plan: included profiles first, then the profile's own jobs,
each job once. It runs the plan on the host step by step, in order, under one approval.

**Why this priority**: it is the gap that makes Keel unable to provision a fresh box. Everything
else in this spec refines it.

**Independent Test**: Use a fixture repository whose `keel-node` profile includes `debian`.
Converge an enrolled host with profile `keel-node`. The plan shows `debian` then `keel-node`. One
approval runs both in order on that host, and the run page shows each step's log and outcome.
Make the first step fail: the second shows `skipped: dependency failed`. Retry: the converge
resumes at the first step and does not re-run steps that succeeded.

**Acceptance Scenarios**:

1. **Given** a host with profiles `keel-node` and `k3s-agent` that both include `debian`,
   **When** it is converged, **Then** `debian` runs once, first, followed by the remaining steps
   in recipe order.
2. **Given** a converge across three hosts, **When** a step fails on one host, **Then** that
   host's later steps are skipped and the other two hosts finish.
3. **Given** a converge waiting for approval, **When** the approver opens it, **Then** they see
   every host and step, the pinned commit, each step's params and credentials (by name), and
   approve or deny the whole plan once.
4. **Given** a converge running on a host, **When** another run for that host is queued,
   **Then** it does not start between the converge's steps. The host is held for the whole chain.
5. **Given** a signed-mode binding, **When** a converge is planned, **Then** every step uses the
   same pinned commit, and the run goes through signed-mode review once, before approval, like a
   single run.
6. **Given** a recipe that includes itself directly or through another profile, **When** the
   manifest is synced, **Then** the manifest is refused with the cycle named, and the previous
   job set stays active.

---

### User Story 2 - New hosts converge on enrollment (Priority: P1)

As an operator, when a host enrolls (single-use token, or an accepted join request), Keel starts
a converge for its profiles without anyone pressing a button. The approval policy still decides
whether a human must approve it.

**Why this priority**: onboarding a fresh box (debian, then k3s-agent, then keel-relay, say) is
the reason the owner asked for this spec.

**Independent Test**: Enroll a fixture runner with profiles `keel-node` and `debian`, with
auto-converge on. A converge appears for that host alone. Under a policy that needs approval it
waits in `pending_approval`; under one that does not, it runs. Enroll a second host with the box
unticked: no converge appears.

**Acceptance Scenarios**:

1. **Given** auto-converge is on (the default), **When** a host completes enrollment, **Then**
   one converge for that host's profiles is created by the actor `system:enroll`, audited, and
   routed through the approval policy.
2. **Given** the host's runner reports a profile ceiling that excludes one of its assigned
   profiles, **When** the converge is planned, **Then** the steps for that profile are left out,
   and the plan says why. The host would refuse them anyway.
3. **Given** a host enrolled through a join request, **When** the operator accepts it, **Then**
   the same rule applies, using the profiles chosen at acceptance.
4. **Given** the binding has no valid manifest, **When** a host enrolls, **Then** no converge is
   created, and the host page says why.

---

### User Story 3 - Edit a host's profiles, with suggestions from inventory groups (Priority: P1)

As a workspace admin, I can change an enrolled host's profiles. Keel suggests profiles from the
host's inventory groups, using the repository's `groups:` map, and I confirm or ignore each one.

**Why this priority**: today profiles are fixed at enrollment. A host that moves from
`compose_hosts` to `k3s_agents` cannot be given its new profile without re-enrolling.

**Independent Test**: A fixture manifest maps `k3s_agents → k3s-agent`. Move a host into
`k3s_agents` in the fixture inventory and sync. The host page shows "suggested: k3s-agent". Confirm
it through Preview → Confirm with step-up: the host's profiles change, an audit row is written,
the host's certificate is re-issued with the new `profile:` principals, and a converge is offered.

**Acceptance Scenarios**:

1. **Given** an enrolled host, **When** an admin previews a profile change, **Then** the preview
   shows profiles added and removed, any added profile outside the host's last reported ceiling
   (flagged: the host will refuse it), and the jobs the change makes eligible.
2. **Given** the change is confirmed, **When** it commits, **Then** it is audited in the same
   transaction. The host's live session gets a re-issued certificate, or the next session does.
3. **Given** a repository inventory change puts a host into a mapped group, **When** the binding
   syncs, **Then** a suggestion appears on the host and on Hosts. Nothing is assigned until an
   admin confirms.
4. **Given** a host leaves a mapped group, **When** the binding syncs, **Then** Keel suggests
   removing the profile and never removes it on its own.

---

### User Story 4 - Narrow jobs and path triggers without double work (Priority: P2)

As an operator, a change to one role starts the narrow job that applies it (`k3s-audit` for
`roles/k3s_audit/**`), not the whole node profile. A narrow job never runs beside a converge that
already applies it.

**Why this priority**: the operator's repository already has narrow playbooks. Without this, a
role change either reruns the whole node or is not triggered at all.

**Independent Test**: A fixture `k3s-master` job declares `covers: [k3s-audit]`. Queue a converge
of `k3s-master` on a host, then push a change under `roles/k3s_audit/`. The `k3s-audit` run for
that host is marked `covered_by` the converge and does not run. Push again after the converge
finishes: the narrow job runs on its own.

**Acceptance Scenarios**:

1. **Given** a queued or running converge on a host whose steps cover job J at the same or a
   newer commit, **When** a path trigger creates a run of J for that host, **Then** that host's
   part of the run ends `covered` with a link to the converge. It is audited and does not run.
2. **Given** no covering converge, **When** J is triggered, **Then** it queues normally behind
   the per-host lock.
3. **Given** a job that declares `covers`, **When** the manifest is synced, **Then** every
   covered name must be a job in the same manifest, or the manifest is refused.

---

### User Story 5 - Check-mode runs (Priority: P2)

As an operator, I can run any job or converge in check mode (`--check --diff`), see what would
change, and nothing is recorded as applied.

**Why this priority**: `CHECK=1` is how the operator previews changes today.

**Independent Test**: Run a fixture job in check mode. The executor passes `--check --diff`. The
run log shows the diff. The host's applied revision and ledger are unchanged, and the run is
labelled `check` everywhere it is listed.

**Acceptance Scenarios**:

1. **Given** check mode, **When** the run finishes, **Then** the host's applied revision, drift
   and rollback point are unchanged.
2. **Given** a check-mode run, **When** approval and signed-mode rules are applied, **Then** they
   are the same as for a real run. Ansible check mode still executes code: tasks marked
   `check_mode: false` and every `command` module run for real. A policy may relax approval for
   check runs explicitly; nothing relaxes it by default.
3. **Given** a converge in check mode, **When** it runs, **Then** every step is in check mode,
   and a step's failure stops that host as usual.

---

### User Story 6 - Credentials as environment variables, resolved through the secret port (Priority: P2)

As an operator, a job declares the environment variables it needs (`TS_AUTHKEY`, `K3S_URL`,
`K3S_TOKEN`) by credential name. Keel resolves them at dispatch through its `SecretResolver`, and
the executor sets them for `ansible-playbook` only.

**Why this priority**: three of the repository's roles read secrets this way. The executor
already has `env` credentials, but nothing in the manifest binds a variable name to a credential.

**Independent Test**: A fixture job declares `env: { K3S_TOKEN: k3s-token }`, and the credential
is granted to the binding. The playbook sees `K3S_TOKEN`. The value never appears in the run log,
the audit, events or the ledger. Remove the grant: the job shows `blocked: credential missing`
and no run is created.

**Acceptance Scenarios**:

1. **Given** a job's `env` map, **When** the manifest is synced, **Then** each variable name is
   checked against the executor's refusal list (`ANSIBLE_*`, `LD_*`, `PYTHON*`, `KEEL_*`, `PATH`,
   `HOME`, `SHELL`, `IFS`, `ENV`, `BASH_ENV`), and a refused name refuses the manifest.
2. **Given** the secret manager of spec 059 is configured, **When** a credential is resolved,
   **Then** it comes through 059's adapter of the same port, with no change to the manifest.
3. **Given** a converge, **When** steps need different credentials, **Then** each step gets only
   its own.

---

### User Story 7 - A revision is runnable only once the repository's checks passed (Priority: P3)

As an operator, I can require that a commit's checks passed (the forge's commit status, e.g. the
CI context that runs `make lint`) before Keel will run it.

**Why this priority**: useful, but the operator's hooks and CI already catch most problems
before a push.

**Independent Test**: A binding requires status context `lint`. Push a commit whose `lint` status
is pending: runs from it wait in `waiting_checks`. Set it to success: they move on to approval.
Set it to failure: they end `checks_failed`, with the context named.

**Acceptance Scenarios**:

1. **Given** a required context, **When** the status is missing or pending, **Then** runs wait,
   for up to the binding's check timeout (default 30 min), then end `checks_failed: timeout`.
2. **Given** no required context, **When** runs are created, **Then** behaviour is unchanged.
3. **Given** a forge without commit statuses, **When** an admin sets a required context, **Then**
   the preview refuses it and names the forge.

---

### User Story 8 - One-off jobs with parameters (Priority: P3)

As an operator, interactive one-offs become manual-only jobs whose prompt is a declared param,
such as `provision` with `stack`. Targets that are not playbooks run on the host stay outside
Keel.

**Independent Test**: A fixture `provision` job declares `params: { stack: { kind: choice,
from: composes/ } }`. Starting it offers the stack names found under `composes/` at the run's
commit. The run passes `stack` as an extra var, and the playbook never pauses.

**Acceptance Scenarios**:

1. **Given** a job with a `choice` param, **When** it is started, **Then** only values listed
   at the pinned commit are accepted, and anything else is refused before approval.
2. **Given** a playbook that would prompt (`pause` without a value), **When** it runs under
   Keel, **Then** it fails at once rather than hanging. The executor gives the playbook no TTY,
   and the job's timeout still applies.

### Edge Cases

- **A recipe step the host cannot run.** A step names a job whose profiles the host neither has
  nor allows. The plan drops it and says why. A plan left with no steps is not created.
- **The manifest changes while a converge is queued.** The converge keeps the plan and commit it
  was approved with. A new sync never rewrites an approved plan.
- **The host disconnects mid-chain.** The running step follows the lost-runner path, and the rest
  of that host's chain is skipped. Retry resumes from the lost step.
- **Keel restarts mid-chain.** Startup recovery (067 D2) marks the in-flight step lost, and the
  same rule applies.
- **Two converges for one host.** The second is refused with a link to the first. Or, if the
  first is still queued and unapproved, the second replaces it, with an audit row.
- **A profile change while a converge is queued.** The queued plan is unchanged. The host page
  offers to re-plan.
- **The group map names a profile the manifest does not declare.** The manifest is refused.
- **A host belongs to several mapped groups.** Every mapped profile is suggested.
- **`group_by` self-enrollment.** The executor forces `--limit` to the host's own inventory
  name and `ansible_connection=local`, so a multi-host refusal guard never fires under Keel.
  Hosts not yet in a group still converge, because their profile, not the inventory, decides
  which steps they get.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The manifest MUST let each profile declare a recipe: `includes` (other profiles,
  applied first, in order) and `jobs` (applied next, in order). A profile without a recipe keeps
  today's meaning, so existing manifests stay valid.
- **FR-002**: Keel MUST expand a host's profiles into one ordered plan, depth-first:
  includes, then jobs. Each job appears once, at its first position. A cycle MUST refuse the
  manifest and name the cycle.
- **FR-003**: A converge MUST run a host's steps one at a time, in order, on one pinned commit
  and one pinned inventory revision. It MUST hold the per-host lock for the whole chain, so no
  other run starts on that host between steps.
- **FR-004**: When a step fails, is lost or times out, Keel MUST skip that host's remaining steps
  (`skipped: dependency failed`) and let other hosts continue. Retry MUST resume at the first
  step that did not succeed, under the same commit, or under a new plan if the operator asks.
- **FR-005**: A converge MUST go through signed-mode review (when the binding is signed) and the
  approval policy once, for the whole plan. The approval MUST show every host, step, param and
  credential name.
- **FR-006**: Keel MUST create a converge when a host completes enrollment or a join request is
  accepted, unless auto-converge was turned off for that enrollment. The actor MUST be
  `system:enroll`, and the converge MUST be audited.
- **FR-007**: Operators with `Operate` on all of a host's profiles MUST be able to start a
  converge for one host or a target selection, through Preview → Confirm.
- **FR-008**: Workspace admins MUST be able to add and remove an enrolled host's profiles
  through Preview → Confirm with step-up. The change is audited in the same transaction, and
  Keel re-issues the host's certificate so its `profile:` principals match.
- **FR-009**: The manifest MUST be able to map inventory groups to profiles (`groups:`). On sync,
  Keel MUST compute suggested additions and removals per host and show them. It MUST NOT assign or
  remove a profile without an admin's confirmation (FR-008).
- **FR-010**: A job MUST be able to declare `covers`, the jobs whose effect it includes. A path
  run of a covered job for a host with a queued or running converge that covers it at the same or
  a newer commit MUST end `covered` for that host, linked and audited.
- **FR-011**: Any job or converge MUST be startable in check mode. The executor MUST add
  `--check --diff`. A check run MUST NOT change the applied revision, drift, ledger or rollback
  point. It MUST be labelled `check` wherever runs are listed. Approval and signed-mode rules
  MUST apply to it unchanged, unless a policy explicitly matches check runs.
- **FR-012**: A job MUST be able to declare `env: { NAME: credential }`. Names MUST pass the
  executor's refusal list at manifest validation. Values MUST be resolved at dispatch through the
  `SecretResolver` port and passed only to `ansible-playbook`'s environment. They MUST never be
  logged, audited, evented or stored in run records.
- **FR-013**: A binding MAY require named forge commit-status contexts. Runs from a commit MUST
  wait (`waiting_checks`) until every required context is `success`, end `checks_failed` on a
  failure, and time out after the binding's check timeout. This MUST NOT apply to forges without
  commit statuses, which the preview refuses.
- **FR-014**: Job params MUST gain a `choice` kind whose allowed values come from a declared
  list or from the directory names under a repository path at the pinned commit.
- **FR-015**: The executor MUST run `ansible-playbook` with no controlling terminal and stdin
  closed, so a prompting play fails at once instead of hanging.
- **FR-016**: Every new manifest key MUST be optional. A manifest without any of them MUST behave
  exactly as before. Strict parsing (unknown keys refused) still applies.
- **FR-017**: The runner MUST check each step against its local `allowed_profiles`, as it does
  today. Converge never widens what a host accepts.
- **FR-018**: Converge, steps, coverage, check mode, profile changes and suggestions MUST go
  through `keel-protocol` with ts-rs bindings. They are shown on the run page, the host page and
  Hosts.
- **FR-019**: OTel spans MUST cover `converge.plan` (hosts, steps), `converge.step` (job,
  outcome) and `profile.suggest`. The runtime flag `KEEL_CONVERGE` gates auto-converge on
  enrollment and the converge command. With it off, Keel behaves as before.

### Key Entities

- **Profile recipe**: per profile, `includes` and ordered `jobs` (manifest).
- **Group map**: inventory group → profiles (manifest).
- **Converge**: a run of kind `converge`, with a pinned commit, inventory revision, actor, check
  flag and approval. It holds hosts and their ordered steps.
- **Converge step**: host × job × position, with an outcome (`pending`, `running`, `succeeded`,
  `failed`, `lost`, `skipped`, `covered`).
- **Profile suggestion**: host, profile, add or remove, the group that caused it, and when it was
  seen.
- **Required check**: binding × status context, with a timeout.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A fresh Debian host enrolled with profiles `debian` and `keel-node` reaches the
  same state as `make keel-node` in one converge and one approval, with no manual run started.
- **SC-002**: Every `make` profile in `../ansible` that chains playbooks or bundles narrow roles
  is expressible in `.keel/jobs.yaml`. The ansible session confirms the manifest delta
  (contracts/ansible.md).
- **SC-003**: A change confined to one narrow role triggers only its narrow job on the affected
  hosts, and never alongside a converge that covers it.
- **SC-004**: A check-mode converge changes no applied revision, ledger entry or rollback point,
  tested.
- **SC-005**: No credential value appears in any log, event, audit row or database column
  outside the sealed credential store, tested with a canary value.
- **SC-006**: A manifest with none of the new keys behaves exactly as before. The existing
  keel-server, keel-core and keel-runner suites pass unchanged.

## Assumptions

- The operator repository keeps one playbook per job. Composite make targets become recipes; Keel
  does not import playbooks.
- `upgrade` and `clean` stay manual-only jobs. `template` runs a shell script on the hypervisor
  over SSH, so it stays outside Keel.
- `KEEL_TOKEN` is only needed to enroll a runner, and that happens before Keel can run anything on
  the host. A converge of an enrolled host never needs it, so the executor's `KEEL_*` refusal
  stays. `KEEL_DIGEST` is already the job param `keel_server_image_digest` (spec 061). Neither
  needs a new path.
- `--ask-become-pass` has no counterpart: the executor runs with its own sudo rules (spec 055).
- The forge adapter from spec 066 can read commit statuses on Forgejo and GitHub.

## Open Questions

None. The owner settled ordering, failure, approval, enrollment, groups, overlap and scope on
2026-11-17 (see the top of this file).

## Out of Scope

- A general job dependency graph across profiles (`needs:`).
- Running molecule or `make lint` on the Keel server. The server executes nothing. The lint gate
  reads statuses the forge already has.
- Moving SOPS-held values into a secret store (spec 059).
- Proxmox VM templating and creating hosts.
- Converging hosts Keel has not enrolled.

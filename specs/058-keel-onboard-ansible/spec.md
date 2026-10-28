# Feature Specification: Onboard the personal Ansible repository onto Keel

**Feature Branch**: `058-keel-onboard-ansible`

**Created**: 2026-10-27

**Revised**: 2026-10-27 (after the Keel trial branch in `../ansible` and the join-token contract)

**Status**: Draft (ready for planning; implementation waits on the 055 pieces listed under Dependencies)

**Input**: User description: "add a spec to onboard our personal repo later lol". Scope agreed in conversation: bring `../ansible` (hosted on git.boop.ninja) under Keel. Fix the automation group's root-equivalent grants first; add the `keel_runner` role per 055 `contracts/host-install.md`; declare profiles matching the Makefile targets and jobs for each in `.keel/jobs.yaml`; pin trust in `group_vars`; keep IP addresses out of Git; enrol existing hosts one at a time with a baseline approval; move from manual `make <target>` runs to Keel-driven runs while keeping `make` as break-glass; retire the manual `git pull` sudo rules afterwards. Nothing in `../ansible` changes until this spec is implemented.

**Revision input**: The owner's onboarding flow is: build the Keel server with `make keel-server`, then on each host run `KEEL_TOKEN=<token> make keel-node`; after that, a host's profiles, its other settings and its runner upgrades all live in Keel. The Keel executor gets its own account (`keel-exec`) instead of reusing the name of the human `automation` group. Keel signs users in through the owner's Keycloak (auth.boop.ninja). The root grant the executor needs waits for this spec; it is not added ahead of it.

## What changed since the first draft

- **Onboarding is one command per host**, `KEEL_TOKEN=… make keel-node`. The token may be a one-time enrollment token or a join token (055 `contracts/join.md`), told apart by shape. The runner is no longer part of the base `debian` profile; the `keel-node` playbook applies the base itself.
- **Runner upgrades go through Keel**: a `keel-node` job reapplies the runner when its pinned versions change, and the runner restarts only after that job has finished.
- **The executor account is renamed** to `keel-exec`. The human `automation` group stays what it is today. This needs a small amendment to 055 (see FR-002).
- **The Keel server host** is a host like any other, but Keel cannot be the only way to repair it.
- **Keycloak**: Keel's sign-in depends on the `keycloak` host, which is itself one of the hosts to onboard.
- **Profiles** now cover every non-interactive Makefile target, including `host` (Proxmox) and `keel-server`. One-off and interactive targets stay `make`-only or become manual jobs.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Close the paths to root and give the executor its own account (Priority: P1)

As the owner of the Ansible repository, I want the human `automation` group to stop being root-equivalent, and I want Keel's executor to run as a separate account that no human shares, before that executor is allowed to become root. Onboarding Keel must not widen an existing hole, and the one grant Keel does need must not land on a name humans already hold.

**Why this priority**: Today a member of the `automation` group can make root run code through the repository's `.git` directory, or start an arbitrary compose file through a wildcard sudo rule. The executor needs root to apply playbooks. If it ran as `automation`, the account that gets that root grant would share its name and group with humans, and they would get access to the executor's files.

**Independent Test**: On a test host with the current base profile, show that both escalations work. Apply the fixed profile, then show that both fail while the legitimate uses (checking repository status, bringing up a listed compose project) still work. Install the runner and show that the executor account is `keel-exec`, that no human is in any of its groups, and that a member of `automation` cannot read the executor's state or use its root grant.

**Acceptance Scenarios**:

1. **Given** a group member on a host with the fixed profile, **When** they try to place a hook or a command-running setting in the repository's Git metadata, **Then** either the write is refused or no privileged process ever runs it.
2. **Given** a group member, **When** they ask sudo to bring up a compose file outside the approved list, or pass extra arguments, **Then** the request is refused.
3. **Given** a group member, **When** they bring up a compose project that is on the approved list, **Then** it starts as before.
4. **Given** a host with the runner installed, **When** its accounts are listed, **Then** the executor runs as `keel-exec`, whose primary group is its own, and no human account is a member of it or of any group it uses.
5. **Given** a member of `automation`, **When** they try to read the executor's state directory or to run a command through the executor's root grant, **Then** both are refused.
6. **Given** a host where the prerequisite changes from scenarios 1 and 2 are not yet applied, **When** the runner role would grant the executor root, **Then** the role refuses and says which prerequisite is missing.

---

### User Story 2 - One command onboards a host (Priority: P1)

As the owner, I want to onboard a host by running `KEEL_TOKEN=<token> make keel-node` on it, after which everything about that host (its profiles, its settings, upgrades to its runner) is handled in Keel, so a new machine is a clone of the repository and one command.

**Why this priority**: This is the step that makes a host a Keel host. Everything after it depends on it.

**Independent Test**: On a throwaway VM, clone the repository and run the command once with an enrollment token and once (on a second VM) with a join token. The first host appears in Keel; the second appears as a join request, and accepting it in Keel enrols it. Running the command again without a token changes nothing.

**Acceptance Scenarios**:

1. **Given** a fresh host and a one-time enrollment token, **When** the owner runs the command, **Then** the base profile is applied, the runner is installed and enrols, and the token is not left on disk, in the repository or in any log.
2. **Given** a fresh host and a join token, **When** the owner runs the command, **Then** the host prints its key fingerprint, appears in Keel as a join request showing the same fingerprint, and enrols only after the owner accepts it in Keel.
3. **Given** an enrolled host, **When** the command is run again with or without a token, **Then** only changed versions or trust settings are updated, and the host's record of applied revisions is untouched.
4. **Given** a host where `make keel-node` has never been run, **When** any other target is applied, **Then** nothing Keel-related is installed.
5. **Given** the repository, **When** it is searched, **Then** it contains no IP addresses and no enrollment tokens.
6. **Given** an enrolled host, **When** the owner commits a change to the pinned runner versions, **Then** Keel runs the `keel-node` job on every node, each node's runner restarts only after its job has finished, and no node downloads a binary whose checksum is not the pinned one.

---

### User Story 3 - Profiles and jobs declared in the repository (Priority: P1)

As the owner, I want the repository to declare the same profiles I use today and one job per profile, so that Keel runs what `make <target>` runs today, on the hosts that should get it.

**Why this priority**: Without the manifest Keel has nothing to run. Matching the existing Makefile targets keeps the mental model the owner already has.

**Independent Test**: Sync the repository into Keel. Keel lists the profiles and their jobs, and a change to a gameserver-only file creates runs only for gameserver hosts.

**Acceptance Scenarios**:

1. **Given** the manifest, **When** Keel syncs the repository, **Then** each declared target has a matching profile and job, and running a job on a host has the same effect as running its `make` target there.
2. **Given** a commit that touches only files used by one profile, **When** it is pushed, **Then** only hosts with that profile get a run.
3. **Given** a commit that touches the base role, **When** it is pushed, **Then** every host gets a run, since every profile includes the base.
4. **Given** the `upgrade` and `clean` targets, **When** the manifest is synced, **Then** they exist as jobs that run only when started by hand in Keel, never because of a commit.
5. **Given** the interactive or one-off targets (`provision`, `template`, `tailscale`, the `secrets*` targets, `lint`, `test`), **When** the manifest is synced, **Then** none of them is a Keel job.

---

### User Story 4 - Trust pinned in the repository, signed mode on (Priority: P1)

As the owner, I want the trusted signers (including my own commit-signing key), my approver passkeys, each host group's allowed profiles and the signed-mode requirement to live in the repository's group variables, so that hosts trust what I decided and not what Keel says.

**Why this priority**: This is what makes a compromised Keel server, Keycloak realm or Git account unable to run code on my hosts.

**Independent Test**: With trust pinned and signed mode required, push an unsigned commit: no host runs it until it is reviewed and approved with a passkey. Change an approver in the variables with an unsigned commit: that change itself needs approval under the old trust.

**Acceptance Scenarios**:

1. **Given** signed mode required on a host group, **When** a commit signed by my key is pushed, **Then** it runs without review.
2. **Given** the same, **When** an unsigned commit is pushed, **Then** hosts wait for a reviewed passkey approval.
3. **Given** a commit that changes trust variables, **When** it reaches review, **Then** the review flags it as a trust change, and hosts judge it under the trust they held before it.
4. **Given** a host group whose allowed profiles exclude `prox-vm`, **When** Keel assigns it `prox-vm`, **Then** the host refuses.
5. **Given** someone who can sign in to Keel through Keycloak but holds no approver passkey pinned on the host, **When** they approve a run, **Then** the host refuses the approval.

---

### User Story 5 - Enrol existing hosts one at a time (Priority: P2)

As the owner, I want to bring my existing hosts under Keel one by one, each starting from a baseline I approve, lowest risk first, so that a mistake affects one machine and I can stop at any point.

**Why this priority**: The hosts already run services, including the Git server and the Keycloak that Keel signs in through. A fleet-wide cut-over is the riskiest possible way to do this.

**Independent Test**: Enrol one host, approve its baseline at the commit it was last applied from, run its job with no changes and see nothing change on the host; then continue to the next.

**Acceptance Scenarios**:

1. **Given** an existing host, **When** it is enrolled, **Then** it runs nothing until I approve a baseline commit for it.
2. **Given** a baseline at the commit the host was last applied from by hand, **When** its job runs, **Then** Ansible reports no changes, or only changes I can explain from the diff.
3. **Given** a host that is not yet enrolled, **When** commits are pushed, **Then** it is unaffected and can still be managed with `make`.
4. **Given** the onboarding order, **When** hosts are enrolled, **Then** ordinary compose hosts come first; the hosts Keel itself depends on (`keycloak`, `forgejo`, the Keel server host) come after break-glass has been proven on at least one host; and the Proxmox host and the k3s nodes come last.

---

### User Story 6 - Keep `make` as break-glass, then retire the manual pull path (Priority: P2)

As the owner, I want Keel to become the normal way changes reach my hosts while I can still run `make <target>` locally on a host when Keel, Keycloak or the Git server is down, and I want the old manual `git pull` sudo rules removed once every host is on Keel.

**Why this priority**: A control plane must not become a single point of failure for fixing the hosts it manages, especially the hosts it runs on or signs in through. Leftover privileged rules are attack surface.

**Independent Test**: With Keel stopped, run `make debian` on a host as an administrator and confirm it applies. Restart Keel and confirm the host reports the drift and the next Keel run reconciles it. Repeat with the Keycloak host stopped. After the last host is onboarded, confirm the `git pull` sudo rules are gone.

**Acceptance Scenarios**:

1. **Given** Keel is unreachable, **When** an administrator runs a `make` target on a host, **Then** it applies as it does today.
2. **Given** a break-glass run happened, **When** Keel next sees the host, **Then** the host is shown as changed outside Keel, and the owner decides whether to re-baseline or re-apply.
3. **Given** a Keel run broke the `keycloak` host so that nobody can sign in to Keel, **When** the owner runs that host's `make` target from the box, **Then** it applies without Keel or Keycloak, and Keel sign-in works again afterwards.
4. **Given** every host is enrolled, **When** the cleanup step is applied, **Then** the manual `git pull`, `git status` and `git fetch` sudo rules for the `automation` group no longer exist on any host.

---

### Edge Cases

- A host is reachable by Ansible today but cannot reach Keel's runner port (network or Tailscale not up); the off-LAN k3s server is the known case.
- A host belongs to more than one profile (for example a Proxmox VM that is also a gameserver).
- An existing host has drifted from the repository, so its first Keel run would change things.
- The owner's signing key expires or is replaced partway through onboarding.
- An approver passkey is lost before a second approver exists.
- A break-glass `make` run is done from a commit that Keel has never seen (local, unpushed).
- A compose project on the approved list is renamed or removed.
- A Keel job on the Keel server host restarts Keel mid-run; the run must end in a known state when Keel returns.
- A Keel job on the `forgejo` host takes the Git server down; Keel cannot fetch until it is back, and hosts keep their last applied revision.
- A host has an old `automation` user created by the trial version of the runner role; it must be migrated to `keel-exec` without losing the executor's ledger.
- A join token leaks; rotating it rejects pending requests without touching enrolled hosts.
- The host's inventory name differs from its hostname, so the executor's forced `--limit` matches nothing.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Before the executor is granted root, the base profile MUST stop granting the `automation` group write access to Git metadata that a privileged process uses, or MUST stop privileged processes from using that metadata.
- **FR-002**: Keel's executor MUST run as a dedicated system account, `keel-exec`, with its own primary group, no login shell, and no human member in any of its groups. 055's runner and host-install contracts, the executor unit and the runner role MUST be amended to use it. The human `automation` group MUST keep its current meaning.
- **FR-003**: The base profile MUST replace the wildcard compose sudo rule with one that allows only compose projects on an explicit list, with no caller-supplied paths or extra arguments.
- **FR-004**: The runner role MUST grant `keel-exec` the root access Ansible needs only after FR-001 and FR-003 are in effect on that host, and MUST refuse with a clear message otherwise.
- **FR-005**: `make keel-node` MUST apply the base profile, then install, configure and start the Keel runner following 055's host-install contract. Hosts where it has never been run MUST get nothing Keel-related.
- **FR-006**: The token MUST be supplied only at run time, through `KEEL_TOKEN`. The role MUST accept an enrollment token or a join token, MUST NOT store either in the repository, MUST NOT log either, and MUST remove the token from the host once it is enrolled.
- **FR-007**: With a join token, the role MUST print the host's key fingerprint so the owner can match it against the join request before accepting it in Keel.
- **FR-008**: Runner upgrades MUST happen only by changing the pinned versions and checksums in the repository. A `keel-node` job MUST apply them, and the runner MUST restart only after that job has finished.
- **FR-009**: The repository MUST NOT contain IP addresses; Keel's address MUST be given as a host name or kept in encrypted or ignored variables.
- **FR-010**: The repository MUST declare one Keel profile and one job for each of these targets: `debian`, `prox-vm`, `gameserver`, `host`, `k3s-master`, `k3s-agent`, `keel-server`, `keel-node`. Each job MUST apply the same playbook its target applies.
- **FR-011**: Each commit-triggered job MUST declare which files trigger it, so that a change triggers runs only for hosts whose profiles use the changed files.
- **FR-012**: `upgrade` and `clean` MUST be manual-only jobs. `provision`, `template`, `tailscale`, `secrets*`, `lint` and `test` MUST NOT be Keel jobs.
- **FR-013**: Every host to be onboarded MUST be in the inventory under the name the executor will limit runs to.
- **FR-014**: Trusted signers, approver passkeys, allowed profiles per host group, override quorum and the signed-mode requirement MUST be defined in the repository's group variables.
- **FR-015**: Signed mode MUST be required on every onboarded host. Only throwaway trial hosts may run unsigned.
- **FR-016**: Hosts MUST be onboarded individually, each with a baseline approval, in the order of User Story 5. A host that is not onboarded MUST keep working as it does today.
- **FR-017**: Running a `make` target locally on a host MUST keep working as a break-glass path without Keel, Keycloak or the Git server. Keel MUST show the host as changed outside Keel afterwards.
- **FR-018**: Once every host is onboarded, the manual `git pull`, `git status` and `git fetch` sudo rules MUST be removed.
- **FR-019**: Hosts that ran the trial version of the runner role MUST be migrated to `keel-exec`, keeping the executor's ledger and Git store.
- **FR-020**: The repository's contributor documentation MUST describe onboarding a host, accepting a join request, upgrading runners, break-glass use (including for the Keel server, Keycloak and Git server hosts), and rotating signers, approvers and the join token.

### Key Entities

- **Host group**: A set of hosts in the inventory that share allowed profiles and trust settings.
- **Profile**: One of the declared Makefile targets, as a Keel profile.
- **Job**: The Keel job that applies one profile's playbook; commit-triggered or manual-only.
- **Trust settings**: Signers, approvers, allowed profiles, quorum and signed-mode requirement for a host group.
- **Baseline**: The approved commit a host starts from when it is enrolled.
- **Approved compose list**: The compose projects the `automation` group may bring up.
- **Executor account** (`keel-exec`): The account Keel's executor runs as, holding the one root grant Keel needs.
- **Onboarding token**: An enrollment token (one host, one use) or a join token (many hosts, each accepted in Keel), given as `KEEL_TOKEN`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Neither known path from the `automation` group to root works on any host after the prerequisite change, and no human account can use the executor's root grant.
- **SC-002**: Every host in the inventory is enrolled in Keel, with signed mode required, before the manual pull rules are removed.
- **SC-003**: For each onboarded host, the first Keel run after its baseline reports no unexplained changes.
- **SC-004**: An unsigned commit reaches zero hosts without a reviewed passkey approval.
- **SC-005**: A break-glass `make` run on any host succeeds with Keel stopped, and the host shows as changed outside Keel within 5 minutes of Keel returning.
- **SC-006**: Onboarding a single new host, from a fresh clone to its first clean run, takes one command on the host plus at most one accept in Keel, and under 15 minutes.
- **SC-007**: A runner upgrade reaches every enrolled node from a single commit, with no command run on any host.

## Dependencies

- From 055 (Keel): host runners, signed mode, profiles, host-verified overrides, and the join-token and runner-download work (T137–T149) for the join-token path. The enrollment-token path does not need join tokens.
- A 055 amendment renaming the executor account to `keel-exec` (FR-002), in the contracts, the unit file and the runner role.
- `RestartPreventExitStatus=78` in the runner unit (055 `contracts/join.md`), so a rejected host does not keep knocking.
- Keel signing in through Keycloak at auth.boop.ninja, with a `keel` client.

## Assumptions

- The repository's owner is the only signer and first approver at the start. A second approver passkey is registered before signed mode is required, so that losing one device does not lock the fleet out.
- The owner's existing GPG commit-signing key is the first trusted signer.
- Hosts can reach Keel's runner port over the owner's private network (for example Tailscale).
- The repository's existing conventions hold: root runs Ansible, the owner's user owns what it deploys, and secrets live in encrypted variables.
- The Makefile targets keep their names; if a target is added later, a matching profile and job are added with it when it is non-interactive.
- Being able to sign in to Keel (through Keycloak) never grants the power to run code on a host; that comes only from the trust pinned on the host.
- The trial branch in `../ansible` (`keel-runner-trial`) is the starting point for the runner and server roles, not a separate design.

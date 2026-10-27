# Feature Specification: Onboard the personal Ansible repository onto Keel

**Feature Branch**: `058-keel-onboard-ansible`

**Created**: 2026-10-27

**Status**: Draft (deferred until 055 is implemented)

**Input**: User description: "add a spec to onboard our personal repo later lol". Scope agreed in conversation: bring `../ansible` (hosted on git.boop.ninja) under Keel. Fix the automation group's root-equivalent grants first; add the `keel_runner` role to the base Debian profile per 055 `contracts/host-install.md`; declare profiles matching the Makefile targets and jobs for each in `.keel/jobs.yaml`; pin trust in `group_vars`; keep IP addresses out of Git; enrol existing hosts one at a time with a baseline approval; move from manual `make <target>` runs to Keel-driven runs while keeping `make` as break-glass; retire the manual `git pull` sudo rules afterwards. Nothing in `../ansible` changes until this spec is implemented.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Close the automation group's paths to root (Priority: P1)

As the owner of the Ansible repository, I want the `automation` group to stop being root-equivalent before any Keel component runs as the `automation` account, so that onboarding Keel does not widen an existing hole.

**Why this priority**: Today a member of the group can make root run code through the repository's `.git` directory, or start an arbitrary compose file through a wildcard sudo rule. Putting a job runner on top of that would make every group member a fleet administrator.

**Independent Test**: On a test host with the current base profile, show both escalations work. Apply the fixed profile, then show both fail while the legitimate uses (checking repository status, bringing up a listed compose project) still work.

**Acceptance Scenarios**:

1. **Given** a group member on a host with the fixed profile, **When** they try to place a hook or a command-running setting in the repository's Git metadata, **Then** either the write is refused or no privileged process ever runs it.
2. **Given** a group member, **When** they ask sudo to bring up a compose file outside the approved list, or pass extra arguments, **Then** the request is refused.
3. **Given** a group member, **When** they bring up a compose project that is on the approved list, **Then** it starts as before.

---

### User Story 2 - The base profile installs a Keel runner (Priority: P1)

As the owner, I want applying the base Debian profile with Keel enabled to install and enrol the Keel runner exactly as Keel's host-install contract describes, so every host I manage can be driven from Keel.

**Why this priority**: This is the step that makes a host a Keel host. Everything after it depends on it.

**Independent Test**: Apply the base profile with Keel enabled and a one-time enrollment token to a throwaway VM; the host appears in Keel with the expected profiles, and re-applying the profile changes nothing.

**Acceptance Scenarios**:

1. **Given** a host where Keel is not enabled, **When** the base profile is applied, **Then** nothing Keel-related is installed.
2. **Given** a host with Keel enabled and a token passed on the command line, **When** the base profile is applied, **Then** the runner is installed, enrols, and the token is not left on disk or in the repository.
3. **Given** a host already enrolled, **When** the base profile is applied again, **Then** only changed versions or trust settings are updated, and the host's record of applied revisions is untouched.
4. **Given** the repository, **When** it is searched, **Then** it contains no IP addresses and no enrollment tokens.

---

### User Story 3 - Profiles and jobs declared in the repository (Priority: P1)

As the owner, I want the repository to declare the same profiles I use today (debian, prox-vm, gameserver, k3s-master, k3s-agent) and one job per profile, so that Keel runs what `make <target>` runs today, on the hosts that should get it.

**Why this priority**: Without the manifest Keel has nothing to run. Matching the existing Makefile targets keeps the mental model the owner already has.

**Independent Test**: Sync the repository into Keel; Keel lists the five profiles and their jobs, and a change to a gameserver-only file creates runs only for gameserver hosts.

**Acceptance Scenarios**:

1. **Given** the manifest, **When** Keel syncs the repository, **Then** each Makefile target has a matching profile and job, and running a job on a host has the same effect as running its `make` target there.
2. **Given** a commit that touches only files used by one profile, **When** it is pushed, **Then** only hosts with that profile get a run.
3. **Given** a commit that touches the base role, **When** it is pushed, **Then** every host gets a run, since every profile includes the base.

---

### User Story 4 - Trust pinned in the repository, signed mode on (Priority: P1)

As the owner, I want the trusted signers (including my own commit-signing key), my approver passkeys, each host group's allowed profiles and the signed-mode requirement to live in the repository's group variables, so that hosts trust what I decided and not what Keel says.

**Why this priority**: This is what makes a compromised Keel server or Git account unable to run code on my hosts.

**Independent Test**: With trust pinned and signed mode required, push an unsigned commit: no host runs it until it is reviewed and approved with a passkey. Change an approver in the variables with an unsigned commit: that change itself needs approval under the old trust.

**Acceptance Scenarios**:

1. **Given** signed mode required on a host group, **When** a commit signed by my key is pushed, **Then** it runs without review.
2. **Given** the same, **When** an unsigned commit is pushed, **Then** hosts wait for a reviewed passkey approval.
3. **Given** a commit that changes trust variables, **When** it reaches review, **Then** the review flags it as a trust change, and hosts judge it under the trust they held before it.
4. **Given** a host group whose allowed profiles exclude `prox-vm`, **When** Keel assigns it `prox-vm`, **Then** the host refuses.

---

### User Story 5 - Enrol existing hosts one at a time (Priority: P2)

As the owner, I want to bring my existing hosts under Keel one by one, each starting from a baseline I approve, so that a mistake affects one machine and I can stop at any point.

**Why this priority**: The hosts already run services. A fleet-wide cut-over is the riskiest possible way to do this.

**Independent Test**: Enrol one host, approve its baseline at the commit it was last applied from, run its job with no changes and see nothing change on the host; then continue to the next.

**Acceptance Scenarios**:

1. **Given** an existing host, **When** it is enrolled, **Then** it runs nothing until I approve a baseline commit for it.
2. **Given** a baseline at the commit the host was last applied from by hand, **When** its job runs, **Then** Ansible reports no changes, or only changes I can explain from the diff.
3. **Given** a host that is not yet enrolled, **When** commits are pushed, **Then** it is unaffected and can still be managed with `make`.

---

### User Story 6 - Keep `make` as break-glass, then retire the manual pull path (Priority: P2)

As the owner, I want Keel to become the normal way changes reach my hosts while I can still run `make <target>` locally on a host when Keel is down, and I want the old manual `git pull` sudo rules removed once every host is on Keel.

**Why this priority**: A control plane must not become a single point of failure for fixing the hosts it manages, and leftover privileged rules are attack surface.

**Independent Test**: With Keel stopped, run `make debian` on a host as an administrator and confirm it applies; restart Keel and confirm the host reports the drift and the next Keel run reconciles it. After the last host is onboarded, confirm the `git pull` sudo rules are gone.

**Acceptance Scenarios**:

1. **Given** Keel is unreachable, **When** an administrator runs a `make` target on a host, **Then** it applies as it does today.
2. **Given** a break-glass run happened, **When** Keel next sees the host, **Then** the host is shown as changed outside Keel, and the owner decides whether to re-baseline or re-apply.
3. **Given** every host is enrolled, **When** the cleanup step is applied, **Then** the manual `git pull`, `git status` and `git fetch` sudo rules for the `automation` group no longer exist on any host.

---

### Edge Cases

- A host is reachable by Ansible today but cannot reach Keel's runner port (network or Tailscale not up).
- A host belongs to more than one profile (for example a Proxmox VM that is also a gameserver).
- An existing host has drifted from the repository, so its first Keel run would change things.
- The owner's signing key expires or is replaced partway through onboarding.
- An approver passkey is lost before a second approver exists.
- A break-glass `make` run is done from a commit that Keel has never seen (local, unpushed).
- A compose project on the approved list is renamed or removed.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Before any Keel component is installed, the base profile MUST stop granting the `automation` group write access to Git metadata that a privileged process uses, or MUST stop privileged processes from using that metadata.
- **FR-002**: The base profile MUST replace the wildcard compose sudo rule with one that allows only compose projects on an explicit list, with no caller-supplied paths or extra arguments.
- **FR-003**: The base profile MUST install, configure and enrol the Keel runner when Keel is enabled for a host, following Keel's host-install contract (055), and MUST do nothing Keel-related when it is not.
- **FR-004**: Enrollment tokens MUST be supplied only at run time and MUST NOT be stored in the repository or left on the host after enrollment.
- **FR-005**: The repository MUST NOT contain IP addresses; Keel's address MUST be given as a host name or kept in encrypted variables.
- **FR-006**: The repository MUST declare one Keel profile per current Makefile target (debian, prox-vm, gameserver, k3s-master, k3s-agent) and a job for each that applies the same playbook the target applies.
- **FR-007**: Each job MUST declare which files trigger it, so that a change triggers runs only for hosts whose profiles use the changed files.
- **FR-008**: Trusted signers, approver passkeys, allowed profiles per host group, override quorum and the signed-mode requirement MUST be defined in the repository's group variables.
- **FR-009**: Signed mode MUST be required on every onboarded host.
- **FR-010**: Hosts MUST be onboarded individually, each with a baseline approval, and a host that is not onboarded MUST keep working as it does today.
- **FR-011**: Running a `make` target locally on a host MUST keep working as a break-glass path, and Keel MUST show the host as changed outside Keel afterwards.
- **FR-012**: Once every host is onboarded, the manual `git pull`, `git status` and `git fetch` sudo rules MUST be removed.
- **FR-013**: The repository's contributor documentation MUST describe onboarding a host, break-glass use, and rotating signers and approvers.

### Key Entities

- **Host group**: A set of hosts in the inventory that share allowed profiles and trust settings.
- **Profile**: One of the five current Makefile targets, as a Keel profile.
- **Job**: The Keel job that applies one profile's playbook.
- **Trust settings**: Signers, approvers, allowed profiles, quorum and signed-mode requirement for a host group.
- **Baseline**: The approved commit a host starts from when it is enrolled.
- **Approved compose list**: The compose projects the `automation` group may bring up.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Neither known path from the `automation` group to root works on any host after the prerequisite change.
- **SC-002**: Every host in the inventory is enrolled in Keel, with signed mode required, before the manual pull rules are removed.
- **SC-003**: For each onboarded host, the first Keel run after its baseline reports no unexplained changes.
- **SC-004**: An unsigned commit reaches zero hosts without a reviewed passkey approval.
- **SC-005**: A break-glass `make` run on any host succeeds with Keel stopped, and the host shows as changed outside Keel within 5 minutes of Keel returning.
- **SC-006**: Onboarding a single existing host, from enabling Keel to its first clean run, takes under 15 minutes.

## Assumptions

- Spec 055 (Keel) is implemented and deployed, including host runners, signed mode, profiles and host-verified overrides. This spec is not started before then.
- The repository's owner is the only signer and first approver at the start; a second approver passkey is registered before signed mode is required, so that losing one device does not lock the fleet out.
- The owner's existing GPG commit-signing key is the first trusted signer.
- Hosts can reach Keel's runner port over the owner's private network (for example Tailscale).
- The repository's existing conventions hold: root runs Ansible, the owner's user owns what it deploys, and secrets live in encrypted variables.
- The Makefile targets keep their names; if a target is added later, a matching profile and job are added with it.
- Proxmox hypervisors and k3s nodes are onboarded last, after the profile has been proven on lower-risk hosts.

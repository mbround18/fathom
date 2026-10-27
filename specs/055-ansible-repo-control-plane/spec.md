# Feature Specification: Keel (Git-Bound Ansible Control Plane)

**Feature Branch**: `055-ansible-repo-control-plane`

**Created**: 2026-10-26

**Status**: Draft

**Input**: User description: "spec a new app similar to AWX/Ansible Tower to manage Ansible nodes from source control, where an admin logs in, sets up a receiver, and binds it to the interface." Follow-up direction: Keel should include Depths-level security posture, full team/user management, and inventory provisioning support with rollback/migration behavior. Brand name finalized as **Keel**. Follow-up direction (2026-10-27): runners live on the managed hosts themselves, installed by the base Debian profile of the operator's Ansible repository and executing as its automation account; runners talk to Keel over SSH with Keel acting as an SSH certificate authority; an optional signed mode refuses unsigned commits unless an approver reviews the diff and approves it with a passkey that the host itself verifies; Keel tracks which revision each host has applied; hosts are assigned profiles, and user roles can be scoped to profiles.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Bind Repository and Receiver (Priority: P1)

As a platform administrator, I can sign in, register a source-control repository as the system of record, configure one or more inbound receivers for repository-driven automation events, and bind those receivers to the management interface so node operations can be controlled from approved repository changes.

**Why this priority**: Without a trusted repository binding and receiver setup, the platform cannot safely orchestrate any managed-node actions.

**Independent Test**: Can be fully tested by signing in as an administrator, creating a repository binding, creating a receiver, linking the receiver to a workspace in the interface, and confirming the platform accepts only events tied to that binding.

**Acceptance Scenarios**:

1. **Given** an authenticated administrator, **When** they register a repository and save receiver details, **Then** the platform persists the binding and shows it as active in the interface.
2. **Given** a configured receiver, **When** the administrator binds it to a workspace or project in the interface, **Then** the binding is visible and eligible to trigger managed automation actions.
3. **Given** an event from an unbound repository or receiver, **When** it reaches the platform, **Then** the platform rejects it and records the refusal reason.

---

### User Story 2 - Drive Node Operations from Source Control (Priority: P2)

As an operations engineer, I can trigger and monitor node-targeted automation runs that originate from repository changes so managed nodes stay aligned with declared configuration.

**Why this priority**: The core value is repository-driven control of Ansible-managed nodes with clear execution visibility.

**Independent Test**: Can be fully tested by using a bound repository event to start a run against a selected node group and confirming run status, progress, and outcomes are visible in the interface.

**Acceptance Scenarios**:

1. **Given** a valid repository-driven trigger, **When** a run starts for selected nodes, **Then** the interface shows queued, running, and terminal states for that run.
2. **Given** a run with mixed node outcomes, **When** execution completes, **Then** the interface shows per-node success/failure results and summary counts.
3. **Given** a run failure caused by unreachable nodes, **When** the run ends, **Then** operators can view actionable failure context for affected nodes.

---

### User Story 3 - Govern Access, Approvals, and Auditability (Priority: P3)

As a security-conscious organization, we can enforce role-based access and approval checkpoints for sensitive automation actions, with complete activity history tied to actor, repository context, and node targets.

**Why this priority**: Operational trust depends on controlled access and traceability for node-changing actions.

**Independent Test**: Can be fully tested by applying role restrictions, requiring approval on sensitive runs, and verifying audit records for denied, approved, and executed actions.

**Acceptance Scenarios**:

1. **Given** a user without required privileges, **When** they attempt a restricted action, **Then** the platform denies the request and records the denial.
2. **Given** a run marked as sensitive, **When** an authorized approver reviews it, **Then** the run executes only after explicit approval.
3. **Given** completed or denied actions, **When** an administrator reviews history, **Then** each record includes actor, time, repository reference, target scope, and outcome.

---

### User Story 4 - Provision and manage inventory safely (Priority: P1)

As an operator, I can provision and manage Ansible inventory from approved repository content or controlled UI workflows so target groups, credentials, and variables stay consistent and auditable.

**Why this priority**: Inventory correctness is foundational to safe node operations.

**Independent Test**: Create/update inventory via repository-driven change, run against the new inventory, and verify inventory lineage and execution outcomes.

**Acceptance Scenarios**:

1. **Given** an approved repository change for inventory, **When** Keel syncs it, **Then** inventory updates are versioned and attributed.
2. **Given** host/group variable updates, **When** they are applied, **Then** validation occurs before any automation run can consume them.
3. **Given** inventory credential references, **When** runs execute, **Then** credentials are resolved at runtime without plaintext storage in audit/history.

---

### User Story 5 - Support rollback and migration workflows (Priority: P2)

As an operator, I can roll back failed or undesired run/inventory changes and migrate inventory/control data across environments with provenance.

**Why this priority**: Recovery and portability are required for production control planes.

**Independent Test**: Run a failing automation workflow, trigger rollback, then execute a migration from one Keel environment to another and verify data integrity and audit continuity.

**Acceptance Scenarios**:

1. **Given** a failed run, **When** rollback is available, **Then** Keel executes rollback steps and records final resulting state.
2. **Given** inventory migration request, **When** migration completes, **Then** source revision, destination revision, and integrity checks are auditable.
3. **Given** migration conflict, **When** collision is detected, **Then** Keel refuses unsafe overwrite and provides conflict details.

---

### User Story 6 - Depths-parity team/user governance (Priority: P1)

As a platform admin, I can apply owner/admin/user roles, team-scoped authorization, immediate revocation, and immutable audit controls equivalent to Depths governance.

**Why this priority**: Keel controls node-changing actions and must meet the same governance bar as Depths.

**Independent Test**: Configure teams/roles, verify allowed and denied operations by scope, change a role, and confirm enforcement updates immediately.

**Acceptance Scenarios**:

1. **Given** a user assigned to one team scope, **When** they request actions outside scope, **Then** access is denied and logged.
2. **Given** an admin removes a user or role, **When** that user acts again, **Then** active sessions are invalidated and access is refused.
3. **Given** sensitive run approval policy, **When** a user lacks approver role, **Then** approval action is denied and audited.

---

### User Story 7 - Enrol hosts through the base profile (Priority: P1)

As an operator, I can bring a host under Keel by applying my repository's base Debian profile to it. The profile installs the Keel runner, which enrols with a one-time token, runs only as the automation account, and only ever executes jobs for its own host.

**Why this priority**: Host runners are where automation executes. If enrollment or the runner's privilege boundary is weak, nothing else in Keel is safe.

**Independent Test**: Apply the base profile to a fresh Debian host with an enrollment token, confirm the host appears in Keel with its profiles, run a job against it, then revoke it and confirm it can no longer connect.

**Acceptance Scenarios**:

1. **Given** a fresh Debian host and a valid enrollment token, **When** the base profile is applied, **Then** the runner is installed, enrolls, and the host appears in Keel with the profiles named in the token.
2. **Given** an enrolled host, **When** Keel dispatches a job, **Then** the runner executes it only against its own host, as the automation account, and reports per-task results.
3. **Given** a runner talking to a server whose identity does not match the one pinned at enrollment, **When** it connects, **Then** it refuses to send or execute anything.
4. **Given** an administrator revokes a host, **When** its runner reconnects or its certificate expires, **Then** Keel refuses it and the refusal is audited.
5. **Given** a job from a profile the host does not allow locally, **When** it is dispatched, **Then** the host refuses it, even if Keel asked.

---

### User Story 8 - Signed mode with reviewed overrides (Priority: P1)

As a security-conscious owner, I can turn on signed mode for a binding so Keel will not run Ansible from commits that are not signed by a trusted signer, unless an approver reviews the exact diff and approves it with a passkey. Each host checks this for itself, so a compromised Keel server still cannot make hosts run unsigned code.

**Why this priority**: Repository content is code that runs as root on managed hosts. Signatures and host-side verification are what make a stolen Git account or a compromised Keel insufficient to take over the fleet.

**Independent Test**: Enable signed mode, push a signed commit (runs), push an unsigned commit (waits for review), review and approve it with a passkey (runs), then send a forged approval directly to a host (refused).

**Acceptance Scenarios**:

1. **Given** signed mode, **When** every commit since the host's last applied revision is signed by a trusted signer, **Then** the run proceeds through normal policy.
2. **Given** signed mode, **When** any commit in that range is unsigned or signed by an unknown key, **Then** the run waits in review, and the interface shows which commits failed and why.
3. **Given** a run waiting in review, **When** an approver opens it, **Then** they see the full diff from each target host's last applied revision to the new commit, file by file, with the commits, authors and signature status.
4. **Given** an approver approves with a passkey, **When** the host receives the job, **Then** the host verifies that approval against approver keys pinned on the host itself before executing.
5. **Given** an approval that was not made by a pinned approver, or that covers a different commit, diff base or host, **When** it reaches a host, **Then** the host refuses the job and reports why.

---

### User Story 9 - Host profiles, profile-scoped roles and revision tracking (Priority: P2)

As an administrator, I can assign hosts to profiles (for example debian, prox-vm, gameserver, k3s-agent), give users roles that apply only to certain profiles, and see which revision of the repository each host has applied for each job.

**Why this priority**: Real fleets mix very different machines. Operators need to know what each host is running and to delegate safely by kind of machine.

**Independent Test**: Assign two hosts different profiles, give a user operator rights on one profile only, confirm they can run jobs there and are refused elsewhere, then confirm each host's applied revision and drift are shown.

**Acceptance Scenarios**:

1. **Given** a host with assigned profiles, **When** a push changes files used by one profile, **Then** only hosts with that profile get a run.
2. **Given** a user with a role scoped to a profile, **When** they act on a host outside that profile, **Then** the action is denied and audited.
3. **Given** completed runs, **When** an operator opens a host, **Then** they see, per job, the applied commit, when, by which run, whether it was signed or overridden, and how far it is behind the branch head.

---

### Edge Cases

- Repository events arrive duplicated or out of order.
- Receiver credentials are rotated or invalidated while bindings are active.
- A repository binding is disabled while runs are queued.
- Target node inventory changes between trigger time and execution time.
- Partial network outages affect only a subset of managed nodes.
- Approval is granted after the run request has expired.
- Rollback artifacts are missing or stale for a requested recovery action.
- Inventory migration destination already contains conflicting host/group identities.
- Team membership changes while a long-running run is in progress.
- A host is offline longer than its SSH certificate's lifetime.
- A host's local record of its last applied revision disagrees with Keel's.
- A signed merge commit brings in unsigned commits.
- The first run on a host in signed mode, when it has no applied revision yet.
- An approver's passkey is replaced, but hosts still pin the old one.
- A job's playbook changes the runner's own installation or trust files.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST require authenticated sign-in before allowing access to administrative setup functions.
- **FR-002**: System MUST allow administrators to register, view, update, disable, and remove repository bindings.
- **FR-003**: System MUST allow administrators to create, rotate, disable, and remove inbound receivers used for repository-driven triggers.
- **FR-004**: System MUST allow administrators to bind receivers to specific interface workspaces/projects and unbind them when needed.
- **FR-005**: System MUST validate that incoming triggers map to an active repository binding and active receiver binding before any run is created.
- **FR-006**: System MUST reject unauthorized or invalid triggers and present a clear refusal reason to authorized users.
- **FR-007**: System MUST allow authorized users to define node target scopes for automation runs, including single-node and multi-node groups.
- **FR-008**: System MUST create a run record for each accepted trigger, including repository context, requested action, target scope, and requesting identity.
- **FR-009**: System MUST display run lifecycle states (queued, running, succeeded, failed, cancelled, denied) in the interface.
- **FR-010**: System MUST provide per-node execution outcomes and a run-level summary for each completed run.
- **FR-011**: System MUST support policy-based approval requirements for sensitive run types before execution begins.
- **FR-012**: System MUST enforce role-based permissions for setup, approvals, run execution, and run cancellation actions.
- **FR-013**: System MUST record immutable audit entries for setup changes, trigger handling decisions, approvals, run actions, and permission denials.
- **FR-014**: System MUST provide searchable history by repository, node scope, actor, time range, and outcome.
- **FR-015**: System MUST preserve operational continuity by preventing new runs from disabled bindings while retaining historical records.
- **FR-016**: System MUST provide owner/admin/user roles and team-scoped authorization with deny-by-default policy.
- **FR-017**: System MUST support immediate session invalidation when users are removed or role/scope changes revoke access.
- **FR-018**: System MUST meet Depths-parity secure defaults: authenticated-only access, encrypted transport, least-privilege service identity, immutable audit records.
- **FR-019**: System MUST support inventory provisioning and lifecycle management (create, update, validate, deprecate) from repository-controlled definitions.
- **FR-020**: System MUST validate inventory and variable correctness before permitting dependent automation runs.
- **FR-021**: System MUST support secret/credential references for inventory and runs without persisting plaintext credential material.
- **FR-022**: System MUST support rollback workflows for run outcomes and inventory mutations where rollback policies are defined.
- **FR-023**: System MUST support migration workflows for inventory/control data between Keel environments with provenance and integrity checks.
- **FR-024**: System MUST preserve repository and inventory lineage in all run, rollback, and migration records.
- **FR-025**: Keel MUST run fully standalone, with its own sign-in, authorization, approvals, and audit, and no UwU Squad portal, event transport, or shared database configured.
- **FR-026** *(delivered by the suite integration spec; 055 keeps the seams, see research R14)*: Keel MAY be enrolled in the UwU Squad suite. When enrolled, it MUST follow the Standalone and Suite Mode rules of spec 057: authorization decided locally from local bindings plus the last verified suite bundle, binding origin recorded, audit forwarded with its own log as source of truth, and workflow steps accepted only through its public API.
- **FR-027** *(suite integration spec)*: Keel approval policies (FR-011) MUST apply to steps invoked by suite workflows; a portal approval MUST NOT replace them.
- **FR-028** *(suite integration spec)*: Keel MUST publish a scope mapping from suite scopes to its workspaces/projects when enrolled.

- **FR-029**: Keel MUST execute Ansible only through runners installed on the managed hosts. Each runner MUST execute jobs only against its own host and MUST NOT be directed at any other host.
- **FR-030**: The runner MUST be installable by the operator's base host profile, MUST execute jobs as the automation account, and its network-facing process MUST NOT hold elevated privileges.
- **FR-031**: Runners MUST connect outbound only, over SSH, authenticating with a host key enrolled once through a single-use, time-limited token, and holding a short-lived SSH certificate issued by Keel. Runners MUST verify Keel's identity against a key pinned at enrollment.
- **FR-032**: Administrators MUST be able to revoke a host runner, which MUST end its connection immediately and refuse all later connections.
- **FR-033**: Keel MUST support an optional signed mode per binding. In signed mode, every commit between a host's last applied revision and the target commit MUST carry a valid signature from a trusted signer before a run may execute, without review. OpenPGP and SSH signatures MUST both be supported.
- **FR-034**: In signed mode, a run that fails the signature check MUST wait for review. The review MUST show the full diff from each target host's last applied revision, file by file, with commit authors and signature status. An override MUST be approved with a passkey and bound to the exact commit, diff base, job and hosts.
- **FR-035**: Hosts MUST independently verify signatures and overrides against trust material pinned on the host (trusted signers, approver passkeys, Keel's keys). A host MAY require signed mode locally regardless of Keel's setting. A compromised Keel server MUST NOT be able to make a host execute an unsigned, unapproved commit.
- **FR-036**: Keel and each host MUST record the applied revision per host and job. Keel MUST show, per host, the applied commit, time, run, signature or override status, and drift from the branch head.
- **FR-037**: Administrators MUST be able to define host profiles and assign hosts to them. Hosts MUST refuse jobs for profiles they do not allow locally.
- **FR-038**: Role grants MUST be scopable to profiles within a workspace, so that a user can hold operator or approver rights for some profiles only.

### Key Entities *(include if feature involves data)*

- **Repository Binding**: A trusted linkage between a source-control repository and a managed workspace/project, including status and ownership metadata.
- **Receiver**: An inbound trigger endpoint definition used to accept repository-originated events and associate them to a binding context.
- **Workspace/Project**: A logical operational boundary in the interface where bindings, policies, and node scopes are managed.
- **Node Scope**: A selected set of managed Ansible nodes targeted by a run.
- **Inventory Definition**: Versioned hosts, groups, and variable structures used for run targeting.
- **Credential Reference**: Indirect secret handle resolved at runtime for inventory or execution access.
- **Automation Run**: A lifecycle record for one repository-driven operation against a node scope.
- **Rollback Workflow**: Ordered recovery actions and their execution outcomes for failed/undesired changes.
- **Migration Workflow**: Controlled transfer process for inventory/control-plane data between environments.
- **Team Membership**: User-to-team mapping that controls accessible inventories, runs, and approvals.
- **Approval Policy**: Rules defining when explicit human approval is required before execution.
- **Audit Record**: Immutable event history capturing actor, action, context, decision, and outcome.
- **Host Runner**: The Keel agent on a managed host, with its enrolled SSH key, current certificate, allowed profiles and status.
- **Host Profile**: A named kind of machine (for example debian, gameserver, k3s-agent) that selects which jobs apply to a host.
- **Trusted Signer**: A public key (OpenPGP or SSH) whose commit signatures satisfy signed mode for a workspace.
- **Signed-Mode Override**: A passkey-signed approval of an exact diff, verifiable by hosts.
- **Applied Revision**: The commit a host last applied for a job, with the run, time and how it was authorized.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 95% of administrators can complete initial repository + receiver binding setup in under 10 minutes on first attempt.
- **SC-002**: 99% of valid repository-driven triggers are accepted or refused with a visible decision in under 15 seconds.
- **SC-003**: 95% of runs provide complete per-node outcome visibility within 30 seconds after execution ends.
- **SC-004**: 100% of denied, approved, started, cancelled, and completed run actions are present in activity history with actor and timestamp.
- **SC-005**: At least 90% of operators report they can determine run status and next action without external tooling.
- **SC-006**: 100% of denied out-of-scope team actions are rejected and auditable.
- **SC-007**: 95% of valid inventory migrations complete with integrity checks and no orphaned references.
- **SC-008**: Keel passes its full test suite with no suite portal, event transport, or shared database configured.
- **SC-009**: 100% of jobs built from unsigned commits in signed mode are refused by hosts unless they carry a host-verifiable passkey override, including when the server is replaced by a hostile one in testing.
- **SC-010**: A fresh Debian host goes from base profile applied to enrolled and runnable in under 5 minutes.
- **SC-011**: For every host, the applied revision per job is visible within 30 seconds of a run ending.

## Assumptions

- Administrators already have a supported identity provider and can assign roles for platform access.
- Managed nodes are Debian hosts brought under Keel by the operator's own Ansible repository (`../ansible`): its base profile installs the runner and pins the host's trust material. Hosts that cannot run a runner are out of scope for 055.
- Before the runner runs as the automation account, that account must not already be root-equivalent: the base role's group-writable `/root/ansible/.git` and wildcard `docker compose` sudo rule must be fixed in the Ansible repository (research R20).
- Any job that uses Ansible privilege escalation is root on its host. Keel's protection is at the gate (signatures, approvals, host verification), not inside an approved job.
- Repository ownership, branching strategy, and change review process are managed outside this feature.
- This feature focuses on repository-bound node orchestration and governance; unrelated infrastructure provisioning workflows are out of scope.

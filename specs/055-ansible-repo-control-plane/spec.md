# Feature Specification: Keel (Git-Bound Ansible Control Plane)

**Feature Branch**: `055-ansible-repo-control-plane`

**Created**: 2026-10-26

**Status**: Draft

**Input**: User description: "spec a new app similar to AWX/Ansible Tower to manage Ansible nodes from source control, where an admin logs in, sets up a receiver, and binds it to the interface." Follow-up direction: Keel should include Depths-level security posture, full team/user management, and inventory provisioning support with rollback/migration behavior. Brand name finalized as **Keel**.

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
- **FR-026**: Keel MAY be enrolled in the UwU Squad suite. When enrolled, it MUST follow the Standalone and Suite Mode rules of spec 057: authorization decided locally from local bindings plus the last verified suite bundle, binding origin recorded, audit forwarded with its own log as source of truth, and workflow steps accepted only through its public API.
- **FR-027**: Keel approval policies (FR-011) MUST apply to steps invoked by suite workflows; a portal approval MUST NOT replace them.
- **FR-028**: Keel MUST publish a scope mapping from suite scopes to its workspaces/projects when enrolled.

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

## Assumptions

- Administrators already have a supported identity provider and can assign roles for platform access.
- Managed nodes are already onboarded for Ansible-based execution by organizational standards.
- Repository ownership, branching strategy, and change review process are managed outside this feature.
- This feature focuses on repository-bound node orchestration and governance; unrelated infrastructure provisioning workflows are out of scope.

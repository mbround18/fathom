# Feature Specification: Ballast (IaC State + Secrets Control Plane)

**Feature Branch**: `056-ballast-state-secrets`  
**Created**: 2026-10-26  
**Status**: Draft  
**Input**: User direction to spec a Rust-first Ballast app for Terraform/OpenTofu/Pulumi state and secret tracking with redb-per-namespace storage, S3-compatible file storage (RustFS in this environment), managed state-bucket replication for redb, Depths-level security posture, full team/user governance, and trilogy-aligned branding with a single-character mark centered on balance.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Manage state and drift by namespace (Priority: P1)

As a platform operator, I can track IaC workspace state and drift history per namespace so state operations are isolated and auditable.

**Why this priority**: Namespace isolation is the core safety property for Ballast.

**Independent Test**: Create two namespaces, run state import/refresh in one, and verify the other namespace has no read/write impact.

**Acceptance Scenarios**:

1. **Given** namespace `team-a`, **When** a state snapshot is written, **Then** only `team-a` storage is changed.
2. **Given** a drift run in `team-b`, **When** results are saved, **Then** drift records are queryable only under `team-b`.
3. **Given** a failed provider run, **When** the run completes, **Then** raw error output is preserved and attached to that run record.

---

### User Story 2 - Track encrypted secret artifacts safely (Priority: P1)

As a platform operator, I can track SOPS-compatible secret artifacts and lifecycle metadata without storing plaintext secrets in Ballast storage.

**Why this priority**: Secret safety is equal priority with state safety.

**Independent Test**: Import a SOPS-encrypted file, inspect stored records, and confirm no plaintext secret value is persisted in redb.

**Acceptance Scenarios**:

1. **Given** an encrypted secret file, **When** it is registered, **Then** Ballast stores metadata, fingerprint, and lineage references only.
2. **Given** a decrypt/edit/re-encrypt workflow, **When** it is completed, **Then** Ballast records version lineage and actor without plaintext retention.
3. **Given** key-source failure, **When** decryption is attempted, **Then** operation fails with actionable error and no partial plaintext persistence.

---

### User Story 3 - Use one UI model across Terraform/OpenTofu/Pulumi (Priority: P2)

As an operator, I can view state and run outcomes through one consistent experience regardless of IaC engine.

**Why this priority**: Ballast must reduce operational fragmentation across engines.

**Independent Test**: Run one operation each via Terraform, OpenTofu, and Pulumi adapters and verify normalized run, diff, and audit output.

**Acceptance Scenarios**:

1. **Given** three engine adapters, **When** operations complete, **Then** normalized run status mapping is consistent in Ballast UI/API.
2. **Given** engine-specific errors, **When** surfaced, **Then** original engine output is preserved verbatim alongside normalized status.
3. **Given** unsupported engine flags, **When** submitted, **Then** request is rejected before execution with validation details.

---

### User Story 4 - Configure provisioning inputs with rollback and migration (Priority: P1)

As an operator, I can define per-run targets, variables, secret references, and credential profiles for Terraform/OpenTofu/Pulumi, with rollback and migration support that behaves consistently across all three engines.

**Why this priority**: Provisioning safety depends on controlled input handling and recovery paths, not only execution.

**Independent Test**: Run apply-style operations for Terraform/OpenTofu/Pulumi with explicit targets/vars/secret refs/credentials, then trigger a failure and validate rollback workflow and migration workflow records.

**Acceptance Scenarios**:

1. **Given** a prepared run template, **When** an operator sets targets, vars, and secret refs, **Then** Ballast validates inputs before engine execution.
2. **Given** credential profile selection, **When** a run starts, **Then** credentials are injected ephemerally and never persisted as plaintext.
3. **Given** a failed apply, **When** rollback is invoked, **Then** Ballast executes the configured rollback plan and records resulting state.
4. **Given** backend/state migration request, **When** migration completes, **Then** source and destination lineage is preserved and auditable.

---

### User Story 5 - Match Depths security and team/user controls (Priority: P1)

As a platform admin, I can enforce owner/admin/user roles, team-scoped namespace access, session revocation, and immutable audit controls equivalent to Depths security expectations.

**Why this priority**: Ballast manages high-impact infrastructure state and credentials, so governance parity with Depths is required.

**Independent Test**: Configure owner/admin/user roles and teams, run allowed/denied actions across namespaces, revoke a user, and verify policy enforcement plus audit visibility.

**Acceptance Scenarios**:

1. **Given** a user assigned only `team-a`, **When** they access `team-b` namespace state or secrets, **Then** access is denied and audited.
2. **Given** role updates by an admin, **When** the user submits the next request, **Then** authorization uses updated policy immediately without restart.
3. **Given** session revocation, **When** a revoked user performs any action, **Then** the session is refused and all live operations stop.
4. **Given** secret views, **When** non-admin users inspect data, **Then** masked secret behavior follows policy and reveal actions are auditable.

---

### User Story 6 - Align with suite branding and marks (Priority: P2)

As a user of the UwU Squad suite, I can recognize Ballast as a first-class sibling of Fathom/Depths/Keel through the same theme system, icon grammar, and a single-character mark.

**Why this priority**: Naming/brand consistency is required for suite coherence.

**Independent Test**: Render suite navigation with all four products and verify Ballast uses shared tokens/components and its assigned mark.

**Acceptance Scenarios**:

1. **Given** shared design tokens, **When** Ballast UI renders, **Then** typography, spacing, and state color semantics match suite standards.
2. **Given** Ballast branding assets, **When** product header and iconography render, **Then** the primary mark is `衡` and alternates are documented for secondary assets.
3. **Given** accessibility checks, **When** marks/icons are decorative, **Then** ARIA text remains English and equivalent to product name.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Ballast MUST store operational data in per-namespace redb files (`data/namespaces/<namespace>.redb`).
- **FR-002**: Ballast MUST enforce namespace-scoped authorization for all read/write state and secret operations.
- **FR-003**: Ballast MUST provide append-only audit records for state operations, secret lifecycle events, and policy denials.
- **FR-004**: Ballast MUST integrate Terraform/OpenTofu/Pulumi via adapter interfaces behind a shared agnostic core.
- **FR-005**: Ballast MUST never persist plaintext secret values in redb or audit records.
- **FR-006**: Ballast MUST support SOPS-compatible encrypted file tracking with lineage, fingerprints, and key-source metadata.
- **FR-007**: Ballast MUST serialize writes per namespace to satisfy redb single-writer constraints while allowing concurrent reads.
- **FR-008**: Ballast MUST preserve engine-native errors verbatim while also mapping them to normalized run status categories.
- **FR-009**: Ballast MUST support S3-compatible object storage (RustFS in local/self-hosted deployments) for encrypted artifact file storage and redb snapshot replication.
- **FR-010**: Ballast MUST support a managed state bucket for encrypted redb snapshot/restore and migration workflows.
- **FR-011**: Ballast MUST provide provisioning-input models across all three engines for targets, variables, secret references, and credential profiles.
- **FR-012**: Ballast MUST inject runtime credentials ephemerally and MUST NOT persist raw provider credentials.
- **FR-013**: Ballast MUST provide rollback workflows for supported apply/mutate operations across Terraform/OpenTofu/Pulumi and persist rollback outcomes.
- **FR-014**: Ballast MUST provide migration workflows (backend and/or namespace scope) with full provenance and integrity checks.
- **FR-015**: Ballast MUST provide owner/admin/user roles, team management, and namespace-scoped access controls equivalent in strictness to Depths.
- **FR-016**: Ballast MUST require authenticated sign-in and refuse all anonymous operational access.
- **FR-017**: Ballast MUST enforce secure defaults equivalent to Depths posture: encrypted transport, least-privilege service identity, immutable audit, and deny-by-default authorization.
- **FR-018**: Ballast MUST support immediate session invalidation/revocation for removed users and role changes.
- **FR-019**: Ballast MUST inherit shared suite theming primitives used by Fathom/Depths/Keel.
- **FR-020**: Ballast MUST define single-character branding as first-class: primary `衡` (balance), alternates `錨` (anchor), `艙` (hold), `鎮` (steady).
- **FR-021**: Ballast MUST run fully standalone, with its own sign-in, authorization, approvals, and audit, and no UwU Squad portal, event transport, or shared database configured.
- **FR-022**: Ballast MAY be enrolled in the UwU Squad suite. When enrolled, it MUST follow the Standalone and Suite Mode rules of spec 057: authorization decided locally from local bindings plus the last verified suite bundle, binding origin recorded, audit forwarded with its own log as source of truth, and workflow steps accepted only through its public API.
- **FR-023**: Ballast rollback, migration, and secret-reveal controls MUST apply to steps invoked by suite workflows; a portal approval MUST NOT replace them.
- **FR-024**: Ballast MUST publish a scope mapping from suite scopes to its namespaces when enrolled.

### Key Entities *(include if feature involves data)*

- **Namespace Store**: A redb database file and schema for one namespace boundary.
- **State Snapshot**: Versioned metadata about workspace state and drift results.
- **Secret Artifact**: SOPS-compatible encrypted file descriptor with fingerprint and lineage.
- **Credential Profile**: Provider access profile containing non-secret references and policy constraints for runtime credential injection.
- **Engine Adapter**: Terraform/OpenTofu/Pulumi execution adapter implementing shared interfaces.
- **Run Record**: Normalized operation lifecycle record with native engine output.
- **Rollback Plan**: Engine-specific recovery strategy and execution record for failed/undesired changes.
- **Migration Plan**: Verified state/artifact transfer descriptor between backends or namespace stores.
- **Team Membership**: Mapping of users to teams and namespace permissions.
- **Audit Event**: Immutable actor/action/outcome entry for all sensitive operations.
- **Brand Mark Set**: First-class Ballast character marks (primary and alternates) with usage semantics.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of write operations are confined to exactly one namespace redb file per request.
- **SC-002**: 0 confirmed plaintext secret-value persistence findings in storage and audit logs.
- **SC-003**: 99% of redb snapshot replications to managed state bucket complete without data-integrity errors.
- **SC-004**: 95% of operator workflows can switch between Terraform/OpenTofu/Pulumi without changing Ballast UI flow.
- **SC-005**: 100% of denied cross-team namespace actions are rejected and audited.
- **SC-006**: 100% of Ballast product surfaces use shared suite theme tokens and the primary mark `衡`.
- **SC-007**: Ballast passes its full test suite with no suite portal, event transport, or shared database configured.

## Branding Notes

- Suite marks: Fathom `尋`, Depths `淵`, Keel `舵`, Ballast `衡`; portal `澪`.
- Ballast conceptual line: stability, trim, and controlled motion under change.
- Poetic motif (for brand copy): 「潮平兩岸闊，風正一帆懸」 (Wang Wan, Tang) — “When the tide is level, the banks open wide; with a true wind, a lone sail hangs steady.”

# Feature Specification: Centralized secret manager for the suite

**Feature Branch**: `059-suite-secret-manager`

**Created**: 2026-10-27

**Status**: Draft (deferred until Fathom, Depths, Keel and Ballast exist and the suite integration
spec has joined them)

**Input**: User description: "spec a centralized secret management app with k8s, depths, keel, and
ballast integrations when we have the synergy between all the apps". Agreed in conversation:

- It is a fifth app with its own spec, not an extension of Ballast and not a section of the suite
  integration spec.
- Ballast keeps tracking SOPS artifacts and their lineage, and points at secrets held here instead
  of holding values.
- Each app reaches it through the `SecretResolver` port it already has. Without the secret
  manager, every app keeps working standalone.
- Spec only. There is no plan until the four apps and the suite integration spec exist.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Store a secret once, with an owner (Priority: P1)

As a team member with write access to a workspace, I can create a secret, give it a name and an owning
team, and later rotate it, without the value ever appearing in a page, a log or an audit record.

**Why this priority**: Without a central store, every app keeps its own copy. One credential then
lives in several places, and rotating it means finding all of them.

**Independent Test**: Create a secret, rotate it twice, then inspect every record the system keeps
about it. The value appears nowhere except in sealed storage. Each version shows who created it
and when.

**Acceptance Scenarios**:

1. **Given** a member with write access to a workspace, **When** they create a secret, **Then** it
   is stored sealed, shown once to its creator if they ask, and recorded with name, owner team,
   version 1, creator and time.
2. **Given** an existing secret, **When** a member rotates it, **Then** a new version becomes
   current, the previous version stays readable for a stated overlap period, and both events are
   audited.
3. **Given** any page, export, log or audit record, **When** it describes a secret, **Then** it
   shows the name, version and fingerprint only, never the value.

---

### User Story 2 - Apps resolve secrets they are granted, and nothing else (Priority: P1)

As an operator of Depths, Keel or Ballast, I can point an app at the secret manager. The app can
then resolve exactly the secrets granted to it, for the purposes they were granted for.

**Why this priority**: Moving secrets to one place is only worth it if apps can use them without
widening who can read them.

**Independent Test**: Register Keel and Depths as consumers. Grant one secret to Keel only. Keel
resolves it. Depths asking for the same secret is refused and audited. Revoke the grant, and Keel's
next resolution is refused.

**Acceptance Scenarios**:

1. **Given** an app registered as a consumer, **When** it asks for a secret granted to it, **Then**
   it receives the current value. The resolution is audited with the app, the purpose (for
   example, the Keel run or the Depths stack) and the version.
2. **Given** an app, **When** it asks for a secret not granted to it, **Then** the request is
   refused and audited, and the response does not reveal whether the secret exists.
3. **Given** a revoked grant, **When** the app asks again, **Then** it is refused immediately,
   with no grace period.
4. **Given** the secret manager is unreachable, **When** an app needs a secret, **Then** the
   operation that needed it fails with a clear reason. The app never falls back to an older value
   or a local copy unless its owner explicitly configured that.

---

### User Story 3 - Deliver secrets to Kubernetes workloads (Priority: P1)

As a platform operator using Fathom, I can have a secret delivered to a namespace in a managed
cluster, so that workloads get it without anyone copying values by hand.

**Why this priority**: Clusters are where most consumers of these secrets run.

**Independent Test**: Grant a secret to one namespace in a test cluster. It appears there for the
workload, and not in any other namespace. Rotate it, and the cluster copy updates within the stated
time. Revoke the grant, and the cluster copy is removed.

**Acceptance Scenarios**:

1. **Given** a grant to a cluster namespace, **When** it is created, **Then** the secret is
   delivered into that namespace only, labelled with its source and version.
2. **Given** a rotation, **When** the new version becomes current, **Then** the delivered copy
   updates within 2 minutes, and Fathom shows which version each namespace holds.
3. **Given** a revoked grant, **When** it is revoked, **Then** the delivered copy is removed within
   2 minutes, and the removal is audited.
4. **Given** someone edits a delivered copy in the cluster, **When** the secret manager next
   checks, **Then** the drift is reported. It is never silently merged back into the source.

---

### User Story 4 - Ballast references secrets instead of holding them (Priority: P2)

As a Ballast user, I can link an encrypted artifact or an IaC workspace to secrets held in the
secret manager. Ballast's lineage then shows which secret versions a state change used, without
Ballast storing values.

**Why this priority**: It closes the gap between Ballast's tracking and an actual store, and leaves
Ballast's boundary unchanged.

**Independent Test**: Link a Ballast workspace to two secrets and run a plan. Ballast's record
shows the secret names and versions used. Ballast's storage holds no value.

**Acceptance Scenarios**:

1. **Given** a Ballast workspace linked to secrets, **When** a run uses them, **Then** Ballast
   records the secret references and versions, and the secret manager records the resolution.
2. **Given** a rotation, **When** Ballast next shows the workspace, **Then** it says which runs
   used an older version.

---

### User Story 5 - Govern who can see and change secrets (Priority: P2)

As an owner or administrator, I can manage teams, grants and approvals with the same model as
Depths and Keel. Sensitive actions need a fresh passkey step-up.

**Why this priority**: The secret manager is the highest-value target in the suite, so its
governance must be at least as strict as the apps it serves.

**Independent Test**: Scope a team to one workspace. A member reads secret metadata there and is
refused elsewhere. Revealing a value, granting a secret to an app, and changing a policy each
require a passkey step-up. A role change takes effect immediately.

**Acceptance Scenarios**:

1. **Given** a member without access to a workspace, **When** they try any action there, **Then**
   it is refused and audited.
2. **Given** a member about to reveal a value, create a consumer grant, rotate a secret, or change
   a policy, **When** they confirm, **Then** a passkey step-up is required within the step-up
   window.
3. **Given** a policy that requires approval for a secret, **When** someone grants it to a new
   consumer, **Then** the grant waits for a different approver.

---

### User Story 6 - Prove what happened (Priority: P3)

As an auditor, I can search the history of every secret by name, consumer, actor, time and
outcome, and I can verify that the history has not been edited.

**Why this priority**: The other stories produce the records; this one makes them useful and
trustworthy.

**Independent Test**: After stories 1–5, search for one secret's history, export it, and run the
integrity check. Then edit one stored audit record offline, and the check reports it.

**Acceptance Scenarios**:

1. **Given** the audit history, **When** an auditor filters it, **Then** every create, rotate,
   reveal, resolve, grant, revoke and refusal appears with actor, time and outcome, and never a
   value.
2. **Given** an audit record edited outside the app, **When** the integrity check runs, **Then** it
   names the first broken record.

---

### Edge Cases

- A secret is deleted while consumers still hold grants. Deletion is refused until the grants are
  revoked, or it revokes them as part of the confirmed preview.
- The overlap period for a rotated secret expires while a long Keel run is still using the old
  version. The run keeps the value it already resolved, and no new resolution of the old version
  succeeds.
- Two apps are granted the same secret for different purposes. Revoking one grant does not affect
  the other.
- A cluster namespace is deleted. Its grants are marked orphaned and reported, never silently
  re-pointed.
- The sealing key is unavailable at start. The secret manager refuses to serve anything and says
  why. It never starts in a degraded mode that serves unsealed data.
- A consumer's credential is stolen. Revoking that consumer stops all its resolutions at once, and
  the audit trail lists what it resolved and when.
- The suite integration spec is not adopted. The secret manager still works standalone with its own
  sign-in, and apps that do not use it are unaffected.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST store secret values sealed at rest, and MUST keep every past version
  until it is explicitly destroyed.
- **FR-002**: The system MUST NOT write a secret value to any log, audit record, export, error
  message or page, except a deliberate reveal to an authorized person after step-up.
- **FR-003**: The system MUST let owners create, rotate (with a configurable overlap period),
  disable and destroy secrets. Each action MUST be previewed before it is confirmed.
- **FR-004**: The system MUST register consuming apps (Depths, Keel, Ballast and Fathom's cluster
  delivery) as distinct consumers, each with its own credential that can be revoked on its own.
- **FR-005**: The system MUST resolve a secret for a consumer only under an active grant that
  names that consumer, that secret and the purpose. Every other request MUST be refused without
  revealing whether the secret exists.
- **FR-006**: The system MUST audit every resolution with consumer, purpose, secret, version and
  time.
- **FR-007**: The system MUST deliver granted secrets into named Kubernetes namespaces of clusters
  that Fathom manages. It MUST update them on rotation, remove them on revocation, and report
  drift in delivered copies without overwriting the source.
- **FR-008**: Ballast MUST be able to reference secrets by name and version for lineage, without
  receiving or storing values unless a run needs them.
- **FR-009**: Each app's existing `SecretResolver` port MUST get one adapter for the secret
  manager. Each app MUST keep working with its standalone resolver when the adapter is not
  configured.
- **FR-010**: The system MUST use the same governance model as Depths and Keel: owner, admin and
  user roles; workspaces; teams; grants; denial by default; immediate revocation on a role
  change.
- **FR-011**: Revealing a value, rotating, granting to a consumer, changing a policy and destroying
  a secret MUST require a passkey step-up.
- **FR-012**: The system MUST support approval policies that require a second, different person for
  chosen actions on chosen secrets.
- **FR-013**: The system MUST keep an insert-only, hash-chained audit history, and MUST provide an
  integrity check that names the first broken record.
- **FR-014**: The system MUST fail closed. It refuses to serve when it cannot unseal, cannot verify
  a consumer, or cannot write the audit record for a resolution.
- **FR-015**: The system MUST let an owner export and import secret metadata and grants between
  instances, with values transferred only sealed to the destination's key, and never in clear.

### Key Entities

- **Secret**: a named value owned by a team in a workspace, with versions, a current version, an
  overlap period and a state (`active`, `disabled`, `destroyed`).
- **Secret version**: one sealed value, with its fingerprint, creator, time and an optional end of
  overlap.
- **Consumer**: a registered app instance (Depths, Keel, Ballast, cluster delivery) with its own
  credential.
- **Grant**: a consumer's right to resolve a secret for a stated purpose (for example, a Keel job, a
  Depths stack or a cluster namespace), with creator and approvals.
- **Delivery target**: a cluster namespace that receives a granted secret, with the delivered
  version and drift status.
- **Policy**: rules for which actions on which secrets need approval or step-up.
- **Audit record**: an insert-only, chained record of an action or refusal.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: After rotation, every consumer that resolves the secret and every delivery target
  holds the new version within 2 minutes, with no manual copying.
- **SC-002**: A sweep of all logs, audit records, exports and pages after the full scenario set
  finds no secret value.
- **SC-003**: 100% of refused resolutions and refused actions appear in the audit history with
  actor or consumer, time and reason.
- **SC-004**: Revoking a consumer or a grant stops new resolutions immediately. The next request
  after the revocation is confirmed is refused.
- **SC-005**: An operator can move one credential that is currently copied into Depths, Keel and a
  cluster to a single managed secret in under 15 minutes, and rotate it once without touching any
  of the three apps.
- **SC-006**: With the secret manager turned off, each of the four apps passes its own standalone
  test suite unchanged.

## Assumptions

- Fathom, Depths, Keel and Ballast exist, and the suite integration spec has defined shared
  sign-in, workspace mapping and audit forwarding. This spec builds on that and does not redefine
  it.
- The app's name and mark are chosen when it is planned, in line with the suite's nautical
  branding.
- Kubernetes delivery is the only push delivery in scope. Depths, Keel and Ballast pull values
  through their resolvers at the moment they need them.
- Dynamic or short-lived credentials minted per request (database users, cloud tokens) are out of
  scope for the first version. Static secrets with versions and rotation are in scope.
- Secrets that already live in SOPS files tracked by Ballast stay there until their owner moves
  them. Importing them is an explicit, audited action, not an automatic sweep.
- Overlap period default: 24 hours. Step-up window: the same as Depths and Keel.

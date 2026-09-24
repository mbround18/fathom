# Feature Specification: In-App Local Kind Cluster Provisioning

**Feature Branch**: `030-kind-cluster-provisioning`

**Created**: 2026-09-23

**Status**: Draft

**Input**: User description: "have fathom able to install and provision a local cluster supporting KinD so we do it all in app and it's hyper powerful."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See whether my machine is ready for a local cluster (Priority: P1)

A Fathom user who wants a local Kubernetes cluster opens the cluster-management panel and immediately sees whether their machine already has what it needs (the `kind` tool and a running container runtime), without opening a terminal.

**Why this priority**: Every other story depends on the user first understanding whether they can proceed, and prerequisite failures are the most common source of confusion for local Kubernetes tooling. Getting this right prevents wasted attempts and support requests.

**Independent Test**: Can be fully tested by opening the cluster-management panel on a machine with, and separately without, the required tools/runtime installed and running, and confirming the panel accurately reflects each state with actionable guidance.

**Acceptance Scenarios**:

1. **Given** the required tool and a running container runtime are both present, **When** the user opens the cluster-management panel, **Then** the panel shows a ready state and offers to create a new cluster.
2. **Given** the required tool is missing, **When** the user opens the panel, **Then** the panel clearly states it is missing and provides a link/instructions for installing it manually; cluster creation is disabled.
3. **Given** the required tool is present but the container runtime is not running, **When** the user opens the panel, **Then** the panel states the runtime is not running (distinct from "not installed") and cluster creation is disabled.
4. **Given** prerequisites were missing, **When** the user installs them outside the app and returns to (or refreshes) the panel, **Then** the panel re-detects and reflects the ready state.

---

### User Story 2 - See what local clusters already exist (Priority: P1)

A user opens the cluster-management panel and sees a list of local clusters that already exist on their machine, so they know what they have before creating something new or deleting something by mistake.

**Why this priority**: Users returning to the app, or who created clusters via the underlying tool previously, need an accurate inventory before taking any action. This is a read-only foundation the create/delete stories build on.

**Independent Test**: Can be fully tested by seeding zero, one, and multiple existing local clusters and confirming the panel lists exactly those, with an accurate empty state when there are none.

**Acceptance Scenarios**:

1. **Given** no local clusters exist, **When** the user opens the panel, **Then** an empty state is shown inviting them to create one.
2. **Given** one or more local clusters exist, **When** the user opens the panel, **Then** each is listed by name with its current status.
3. **Given** the list is showing, **When** the user triggers a refresh, **Then** the list re-queries the host and reflects any changes made outside the app.

---

### User Story 3 - Create a new local cluster from inside the app (Priority: P1)

A user who wants a disposable local Kubernetes cluster names it, optionally picks a starting profile, reviews a summary of what will be created, confirms, and watches live progress until the cluster is ready to use — never touching a terminal.

**Why this priority**: This is the core value proposition of the feature ("hyper powerful," "all in app") and the reason the feature exists.

**Independent Test**: Can be fully tested end-to-end by entering a valid name, confirming creation, observing progress update over the ~30-90 second duration, and confirming a new cluster appears in both the local cluster list and the app's cluster/context switcher afterward.

**Acceptance Scenarios**:

1. **Given** prerequisites are satisfied, **When** the user opens "create cluster" and enters a valid name, **Then** the app shows a summary (name, profile, node count) and requires explicit confirmation before doing anything on the host.
2. **Given** the user confirms creation, **When** the operation is running, **Then** the app shows a live, updating progress/status view for the duration of the operation and the user can see it is still working rather than assuming the app has frozen.
3. **Given** creation succeeds, **When** the operation finishes, **Then** the new cluster appears in the local cluster list and is immediately selectable as a cluster/context elsewhere in the app, with no separate manual import step.
4. **Given** creation fails partway (e.g. runtime error, name collision), **When** the failure occurs, **Then** the app surfaces a clear error, leaves no confusing partial state hidden from the user, and lets the user retry.
5. **Given** the user enters a name that is not a safe, valid cluster name (empty, too long, invalid characters, reserved words), **When** they attempt to proceed, **Then** the app rejects the name inline with guidance before any confirmation step is reachable.
6. **Given** the user is offered profile choices, **When** they pick a profile other than the bare default, **Then** the confirmation summary reflects the additional components that profile will install.

---

### User Story 4 - Delete a local cluster from inside the app (Priority: P2)

A user who no longer needs a local cluster removes it from the list, after explicitly confirming the destructive action, and it disappears from both the local list and the cluster/context switcher.

**Why this priority**: Necessary for the feature to be a complete lifecycle rather than one-way, but less urgent than creation itself and lower risk if delayed slightly relative to P1s.

**Independent Test**: Can be fully tested by creating a disposable cluster (or using a pre-seeded one), deleting it through the app, confirming the destructive-action prompt appears and blocks the deletion until confirmed, and confirming the cluster is gone from all views afterward.

**Acceptance Scenarios**:

1. **Given** a local cluster exists, **When** the user chooses to delete it, **Then** the app shows an explicit destructive-action confirmation naming the cluster before anything is deleted.
2. **Given** the user confirms deletion, **When** the operation completes, **Then** the cluster no longer appears in the local cluster list or the cluster/context switcher.
3. **Given** the user cancels the confirmation, **When** they back out, **Then** nothing on the host is changed.
4. **Given** the cluster being deleted is the one currently selected/active in the app, **When** deletion completes, **Then** the app gracefully falls back to another available cluster/context or a clear "no cluster selected" state, without crashing or hanging on a now-nonexistent cluster.

---

### Edge Cases

- What happens if the user tries to create a cluster with a name that already exists locally? The app must detect the collision and reject before confirmation (or on host rejection, surface it clearly) rather than silently overwriting or hanging.
- What happens if the container runtime stops running (e.g. Docker Desktop quit) partway through an in-progress create or delete operation? The app must detect the failure and report it clearly rather than showing indefinite progress.
- What happens if the user closes/backgrounds the app while a create or delete operation is in progress? The operation should continue to completion on the host and the app should reflect the correct end state when reopened/refreshed.
- What happens if a local cluster was created or deleted by some other tool (outside the app) while the panel is open? A manual refresh must reconcile the displayed list with reality.
- What happens if prerequisites become unmet (runtime stops, tool is uninstalled) between when the panel loads and when the user attempts to create a cluster? The app must re-validate at the point of action, not only at panel load.
- What happens if two create or delete operations for the same cluster name are attempted concurrently? The app must prevent or clearly serialize concurrent operations against the same cluster name.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST detect and display whether the local cluster tool is installed on the host.
- **FR-002**: The system MUST detect and display whether a supported container runtime is installed and currently running on the host, distinguishing "not installed" from "installed but not running."
- **FR-003**: The system MUST NOT attempt to silently install the local cluster tool or a container runtime on the user's behalf; when either is missing, it MUST present clear, actionable guidance (e.g. what to install and where to find it) instead.
- **FR-004**: The system MUST list all local clusters currently present on the host and allow the user to manually refresh that list on demand.
- **FR-005**: The system MUST show an explicit empty state when no local clusters exist.
- **FR-006**: The system MUST allow the user to specify a name for a new local cluster and validate that name as a safe, DNS-label-style identifier (non-empty, bounded length, restricted character set, no reserved/unsafe values) before allowing the user to proceed to confirmation.
- **FR-007**: The system MUST reject cluster names that collide with an existing local cluster name, with a clear inline message.
- **FR-008**: The system MUST offer at least one starting "profile" choice for a new cluster (at minimum, a bare single-node default) and MUST reflect the chosen profile's implications in the pre-creation summary.
- **FR-009**: The system MUST require the user to review a summary of the action (cluster name, profile, and what will be created) and explicitly confirm before any change is made on the host.
- **FR-010**: The system MUST perform cluster creation as a single tracked operation and MUST show the user continuously updating progress/status for its full duration, distinguishing "in progress," "succeeded," and "failed" end states.
- **FR-011**: Upon successful cluster creation, the system MUST make the new cluster's connection information available through the same mechanism the app already uses to let users select among multiple clusters, so the new cluster is immediately selectable without a separate manual import step.
- **FR-012**: The system MUST allow the user to delete an existing local cluster, gated by an explicit destructive-action confirmation that names the specific cluster being deleted.
- **FR-013**: Upon successful cluster deletion, the system MUST remove the cluster from the local cluster list and from the app's cluster/context selection mechanism.
- **FR-014**: If the cluster being deleted is the one currently active/selected in the app, the system MUST transition to another available cluster or an explicit "no cluster selected" state without error.
- **FR-015**: The system MUST surface clear, specific error messages when a create or delete operation fails or when prerequisites are not met at the moment the user attempts an action, rather than failing silently or hanging indefinitely.
- **FR-016**: The system MUST prevent or clearly serialize concurrent create/delete operations targeting the same cluster name.
- **FR-017**: The system MUST treat local cluster creation and deletion as operations confined to the local machine and its local container runtime; the system MUST NOT provision, list, or delete clusters on any remote host.
- **FR-018**: The system MUST scope this feature to the specific local-cluster tooling family named in this spec (kind-backed clusters) and MUST NOT present or imply support for other cluster distributions or cloud providers.

### Key Entities

- **Local Cluster**: A kind-backed Kubernetes cluster running on the user's own machine via the local container runtime. Key attributes: name (unique on the host), status (e.g. ready, in-progress, error), the profile it was created with, and creation time.
- **Cluster Profile**: A named starting configuration for a new local cluster (e.g. "bare" vs. a profile that pre-installs a small set of common cluster add-ons). Key attributes: display name, description of what it includes.
- **Prerequisite Status**: The detected readiness state of the host for this feature. Key attributes: whether the local cluster tool is present, whether a container runtime is present, whether that runtime is currently running.
- **Cluster Operation**: A tracked, long-running create or delete action against a specific Local Cluster. Key attributes: target cluster name, kind of operation (create/delete), current state (pending confirmation, in progress, succeeded, failed), and progress detail shown to the user.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user with prerequisites already met can go from "no local cluster" to "a running, selectable local cluster" entirely inside the app, without opening a terminal, in under 2 minutes.
- **SC-002**: 100% of destructive delete actions require an explicit confirmation step naming the target cluster before any host-level change occurs.
- **SC-003**: A user on a machine missing a prerequisite can determine exactly what is missing and what to do about it within 10 seconds of opening the cluster-management panel, without consulting outside documentation.
- **SC-004**: During cluster creation, the user sees a progress update at least every few seconds for the duration of the operation, such that at no point does the app appear frozen or unresponsive for longer than a brief moment.
- **SC-005**: A newly created local cluster appears in the app's existing cluster/context switcher and is selectable within 5 seconds of the create operation reporting success, with no separate manual step required.
- **SC-006**: 100% of attempts to create a cluster with an unsafe or colliding name are rejected before any host process is invoked.

## Assumptions

- "The local cluster tool" in this spec refers specifically to kind (Kubernetes-in-Docker); no other local cluster distribution (e.g. minikube, k3d) is in scope.
- The user already has, or is willing to separately install, a supported container runtime (Docker or Podman) and the `kind` tool itself; whether the app may optionally offer to include a small copy of the `kind` tool itself (as distinct from the container runtime) is an open implementation question deferred to the planning phase, not decided by this spec.
- Local clusters are created and managed only against the local container runtime on the same machine running the app; no remote or SSH-based provisioning is in scope.
- A single default "bare" profile is sufficient to ship an initial version of this feature; additional profiles (e.g. pre-installed common add-ons) are valuable but the exact set of add-ons is an implementation decision, not a scope-defining one.
- Advanced users who want full control over multi-node or custom-topology clusters are explicitly out of scope for the primary flow in this version; if a raw-configuration paste-in affordance is offered at all, it is a secondary, optional path and its absence does not block this feature from shipping.
- This feature registers new clusters into the app's existing mechanism for switching between multiple known clusters; it does not introduce a second, competing way of tracking known clusters.
- Users of this feature are expected to be technical operators/developers already using Fathom to manage Kubernetes clusters, not first-time Kubernetes users needing a guided tutorial.

## Out of Scope

- General remote or cloud cluster provisioning (e.g. AWS, GCP, Azure, or any hosted Kubernetes offering).
- Any local cluster distribution other than kind (e.g. minikube, k3d).
- Provisioning or managing clusters on any host other than the machine running the app.
- Automatically installing the container runtime (Docker/Podman) on the user's behalf.
- A custom multi-node topology editor/wizard (e.g. visual editing of HA control-plane layouts or custom CNI configuration) for the initial version of this feature.
- General cluster lifecycle management or "Projects"-style grouping features beyond this local, kind-specific scope.

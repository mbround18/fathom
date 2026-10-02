# Feature Specification: Docker and container-runtime management

**Feature Branch**: `048-docker-management`

**Created**: 2026-10-02

**Status**: Draft

**Input**: User description: "could we spec out docker management as well? its all same vein really and it would do wonders for my local workflow is that pheasible as its own special context if it detects docker running on the system"

## Clarifications

### Session 2026-10-02

- Q: Where does Docker live in the UI? → A: Its own top-level scene, a sibling of Clusters, present only when a runtime is detected. Explicitly **not** a pseudo-context in the kubeconfig switcher: 046 keys a fleet member by context name and defines groups as label selectors, and a container runtime has neither, so a synthetic `docker:local` context would corrupt the fleet model.
- Q: How far does mutation go? → A: Full parity with the container CLI's object operations — lifecycle, removal, creation, build, and project-level up/down. Chosen over a narrower lifecycle-only scope with the larger surface stated as the trade-off.
- Q: What is managed beyond bare containers? → A: All of it — containers grouped by project, images, volumes and networks with their reclaimable space and references, streamed logs and an interactive shell, and live CPU/memory per container and per project.
- Q: Are containers and clusters cross-linked? → A: Yes, both directions. A container that backs a local cluster says which cluster, and a cluster says which containers back it.
- Q: Does creation include a free-text command field? → A: Form-driven only, over documented object fields. A free-text field is arbitrary command execution launched from the UI and its effect cannot be enumerated in advance, so it cannot satisfy the preview guarantee the rest of the feature rests on. Flags no form covers are reached from a terminal, deliberately.
- Q: May Fathom act on projects it did not create? → A: No — read-only unless adopted. Every project is visible, readable, loggable and measurable from the start, and becomes mutable only after the operator adopts it explicitly. A mis-click cannot reach an unrelated project at all.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See what is actually running on this machine (Priority: P1)

An operator whose daemon is shared by several unrelated projects opens Fathom. A Docker section is
present in the sidebar because a runtime was detected and is running. It lists every container
grouped by the project that owns it, so three dozen containers read as four units rather than one
undifferentiated list. Each container shows its state, image, published ports, uptime and health.
Containers with no owning project sit in their own clearly named bucket rather than being hidden or
mixed in.

**Why this priority**: It is the whole feature in its smallest useful form, and it answers the
question the operator asks first and most often — what is running, and whose is it. Every later
story acts on what this one shows. It is also what makes the rest safe: grouping by project is the
unit in which blast radius is later described.

**Independent Test**: With containers from three different projects plus two standalone containers
running, open the scene and confirm four groups appear with the right members, and that each
container's state, image, ports and uptime match what the runtime reports.

**Acceptance Scenarios**:

1. **Given** no container runtime is installed, **When** Fathom starts, **Then** the Docker section is absent from the sidebar entirely — not present-and-empty, and not an error screen.
2. **Given** a runtime is installed but not running, **When** the operator opens the Docker section, **Then** it states that the runtime is installed and stopped, naming which runtime, and does not say "no containers".
3. **Given** containers labelled with three different project names, **When** the scene loads, **Then** three groups are shown, each headed by its project name with its member count and how many of those members are running.
4. **Given** a container carrying no project label, **When** the scene loads, **Then** it appears in a separate group whose heading says these containers belong to no project, and it is never silently attributed to one.
5. **Given** the runtime is stopped while the scene is open, **When** the next refresh occurs, **Then** the scene says the runtime became unavailable with the reason verbatim, and keeps showing the last known list marked as stale rather than blanking it.
6. **Given** the operator's account lacks permission to reach the runtime socket, **When** the scene opens, **Then** it says exactly that, names the socket, and states the remediation, rather than reporting an empty machine.

---

### User Story 2 - Act on a container or a whole project without leaving Fathom (Priority: P2)

The operator has a stuck service. They adopt its project once — declaring that Fathom may act on
it — and from then on they start, stop, restart, pause, unpause or kill a single container, or do
the same to every container in that project at once. Projects they have not adopted stay fully
visible and fully readable, but offer no action at all, so a mis-click cannot reach an unrelated
project. Before anything runs, Fathom shows the exact operation it is about to issue, against the
exact objects, and nothing happens until the operator confirms. The result is reported from what
the runtime actually did, not assumed.

**Why this priority**: It is the daily-workflow win and the reason the feature was asked for. On
its own, with only US1 beneath it, it already replaces the common trip to a terminal.

**Independent Test**: With one project adopted and another not, confirm the unadopted project
offers no action and says why. Then stop a single running container in the adopted project and
confirm the preview named that container and that operation, the container is stopped afterwards,
and the row's state came from the runtime rather than being set optimistically. Then restart the
whole project and confirm every member was named in the preview.

**Acceptance Scenarios**:

1. **Given** a project the operator has not adopted, **When** they view its containers, **Then** every detail is readable and no mutating action is offered, and the absence is explained as "not adopted" with adoption offered — not an unexplained missing control.
2. **Given** an unadopted project, **When** the operator adopts it, **Then** actions become available for every container in it, including containers that join it later, and the adoption survives restarting Fathom.
3. **Given** an adopted project, **When** the operator releases it, **Then** it returns to read-only and no action is offered for it.
4. **Given** a running container in an adopted project, **When** the operator chooses Stop, **Then** the exact operation and the container it targets are shown before it is issued, and the operator must confirm.
5. **Given** the operator confirms, **When** the runtime reports the result, **Then** the row's new state reflects what the runtime reported, and a failure is shown with the runtime's own message verbatim.
6. **Given** an adopted project with four containers, **When** the operator restarts the project, **Then** all four are named in the preview before anything runs, and a per-container outcome is reported for each.
7. **Given** a project-level operation where one container fails, **When** the operation finishes, **Then** the outcome states which succeeded and which failed, and does not report overall success.
8. **Given** a container is removed outside Fathom between the preview and the confirmation, **When** the operation is issued, **Then** the runtime's "no such container" is reported verbatim and no other object is touched.

---

### User Story 3 - Reclaim space without breaking another project (Priority: P3)

The operator is out of disk. Panes for images, volumes and networks show each object's reclaimable
size and what currently references it, so "what is safe to delete" is answerable without guessing.
Removal and prune are available, and each one states in advance every object it will remove, how
much space that frees, and — critically — which projects other than the one being worked on fall
inside that blast radius. Destructive operations require a typed confirmation and are verified
afterwards against what was actually removed.

**Why this priority**: It is the second real pain after "what is running", and it is where a
container UI does the most damage. The operator's daemon is shared by unrelated projects and a
careless prune has already cost real work, so the guard is part of the feature, not a later polish.

**Independent Test**: With dangling images, a tagged image referenced by a stopped container, and
an unused volume present, open the three panes and confirm each object's size and referencing
objects are correct; then issue a prune and confirm the preview lists exactly the objects that are
afterwards gone, names every project touched, and that the operation required a typed confirmation.

**Acceptance Scenarios**:

1. **Given** images exist, **When** the images pane loads, **Then** each row states its size, whether it is tagged or dangling, and which containers reference it, and untagged images are never presented as interchangeable with tagged ones.
2. **Given** a broad prune is selected, **When** the preview is shown, **Then** it names every object that would be removed and the total space reclaimed, and the operation cannot be issued while any object in that list is unnamed or summarised as a count alone.
3. **Given** a prune whose blast radius includes objects belonging to projects other than the one selected, **When** the preview is shown, **Then** those projects are named explicitly as being affected, before the operation can be confirmed.
4. **Given** a destructive operation, **When** the operator confirms, **Then** confirmation requires typing the name of what is being destroyed, and afterwards Fathom verifies and reports what was actually removed against what was predicted, including any discrepancy.
5. **Given** a volume in use by a running container, **When** its removal is previewed, **Then** the preview says it is in use and names the container, and the runtime's refusal is reported verbatim if the operation is issued anyway.
6. **Given** a removal the operator cancels, **When** they cancel, **Then** nothing was issued and nothing changed.

---

### User Story 4 - Containers and clusters are one world (Priority: P4)

The operator runs local Kubernetes clusters, whose nodes are themselves containers on this machine.
A container recognised as a cluster node says which cluster it belongs to and links to that
cluster's views; a local cluster says which containers back it and links back. If the operator is
about to stop or remove a container that backs the cluster they are currently connected to, Fathom
says so before acting, naming the cluster that will break.

**Why this priority**: It is the thing no general-purpose container UI can do, and it is a safety
feature as much as a convenience — the two lists are the same objects seen from two sides, and not
knowing that is how an operator kills the cluster they are working in.

**Independent Test**: With a local cluster running, confirm its node containers are labelled with
the cluster name and link to it, that the cluster's own view lists those containers, and that
stopping one while connected to that cluster produces an explicit warning naming the cluster.

**Acceptance Scenarios**:

1. **Given** a container that is a node of a local cluster, **When** it is listed, **Then** it states which cluster it belongs to and offers a link to that cluster.
2. **Given** a local cluster, **When** the operator views it, **Then** the containers backing it are listed with their current state, and link back to the Docker section.
3. **Given** the operator is connected to a cluster, **When** they preview stopping or removing a container that backs it, **Then** the preview warns that this will break that cluster, names it, and says the operator is currently connected to it.
4. **Given** a cluster node container is stopped outside Fathom, **When** the scene refreshes, **Then** both the container's state and the cluster's own availability reflect it, and neither claims the cluster is healthy.
5. **Given** a container that merely resembles a cluster node but carries no cluster identity, **When** it is listed, **Then** no cluster is claimed for it.

---

### User Story 5 - Read the logs and get a shell, where the cluster ones already are (Priority: P5)

The operator follows a container's logs as a live stream, with the same controls, search and bounded
buffering as the cluster log views, and opens an interactive shell inside a running container the
same way they open one in a workload.

**Why this priority**: It closes the last common reason to leave for a terminal, and it is mostly
reuse: the streaming, buffering, truncation and terminal behaviour already exist for cluster
workloads, so this story is largely about pointing them at a different source.

**Independent Test**: Follow a chatty container's logs and confirm the stream appears live, is
bounded with an explicit truncation marker, and is searchable; then open a shell in that container,
run a command, and confirm the session closes cleanly and stops streaming.

**Acceptance Scenarios**:

1. **Given** a running container, **When** the operator follows its logs, **Then** output appears live, is batched and bounded, and truncation is marked explicitly rather than silently dropping lines.
2. **Given** a stopped container, **When** the operator views its logs, **Then** the retained output is shown and it is stated that the container is not running, with no stream left open.
3. **Given** a running container with a usable shell, **When** the operator opens a session, **Then** input and output behave as in the cluster terminal, and closing the view ends the session with no process left behind.
4. **Given** a container with no usable shell, **When** a session is attempted, **Then** the runtime's own failure is reported verbatim rather than an empty terminal.
5. **Given** the operator leaves the Docker section, **When** they navigate away, **Then** no log stream or session stays open for a view that is no longer visible.

---

### User Story 6 - See what is eating the machine (Priority: P6)

Live CPU and memory are shown per container and aggregated per project, so the operator can tell
which project is responsible for the load before deciding what to stop.

**Why this priority**: Valuable and frequently wanted, but it is diagnosis rather than action — it
tells the operator what to do with US2, which must exist first. It is also the story with the
clearest cost against Principle V, so it comes after the cheap ones are proven quiet.

**Independent Test**: With one container under deliberate load, confirm its usage is visibly higher
than its idle siblings, that its project's aggregate reflects it, and that the figures stop being
collected when the scene is closed.

**Acceptance Scenarios**:

1. **Given** the usage view is open, **When** containers are running, **Then** each shows current CPU and memory, and each project shows the aggregate of its members.
2. **Given** a container is under load, **When** the figures update, **Then** the increase is visible within the same order of latency as cluster metrics views.
3. **Given** the operator closes or navigates away from the Docker section, **When** nothing is visible, **Then** no usage sampling continues.
4. **Given** the runtime cannot report usage for a container, **When** its row renders, **Then** the figure is shown as unavailable with the reason, not as zero.
5. **Given** usage history, **When** it is retained, **Then** it is bounded with an explicit cap, and exceeding it is visible rather than silent.

---

### User Story 7 - Create, run and build from here too (Priority: P7)

The operator creates and starts a new container, builds an image from a context, creates a network
or a volume, and brings a whole project up or down from its own definition file — each one from a
form over the object's documented fields, and each one showing the exact operation before it runs.
There is no free-text command field: everything this feature can issue is something it can also
describe in advance.

**Why this priority**: It completes the parity the operator asked for, but it is the part their
existing definition files and CLI already serve well, and it is the largest surface with the least
daily use. Last, so that nothing above it waits on it.

**Independent Test**: Create a network, create a volume, create and start a container that uses
both, build an image from a small context, and bring a project down and back up — confirming each
operation's preview named exactly what it would do and that the result matches.

**Acceptance Scenarios**:

1. **Given** the operator fills in a new container's settings, **When** they confirm, **Then** the exact operation is shown first, and the created container appears with the settings that were requested.
2. **Given** an image build, **When** it runs, **Then** build output streams live and a failure is reported with the builder's own output, not a generic message.
3. **Given** a project whose definition file is present, **When** the operator brings it up or down, **Then** the preview names the file it will act on and every container the operation affects, and bringing it down is treated as destructive and guarded accordingly.
4. **Given** a project whose definition file is missing or unreadable, **When** project-level operations are offered, **Then** they are unavailable for that project with the reason stated, and the file is never rewritten or deleted.
5. **Given** a creation the runtime rejects, **When** it fails, **Then** the runtime's message is shown verbatim and nothing partially created is left unreported.
6. **Given** a setting no form covers, **When** the operator looks for a free-text command field, **Then** there is none, and the feature says plainly that this case belongs in a terminal rather than offering an unpreviewable escape hatch.

---

### Edge Cases

- The runtime socket exists but the operator's account cannot open it — the scene must say so, naming the socket, rather than reporting an empty machine.
- The runtime is reached over a non-local connection because the environment points elsewhere: the scene must name which daemon it is connected to, so "this machine" is never implied falsely.
- The runtime is rootless, or is a Docker-compatible alternative: object identity, project labels and sizes must come from the runtime rather than being assumed.
- A project's containers are partly running and partly exited — the group heading must state both counts, not just a member count.
- Two containers in different projects share a name; every operation must act on identity, never on a display name.
- A project's definition file was moved or deleted after its containers were created.
- Several hundred containers are present: the list must stay responsive and virtualized.
- An object is removed, or a project is brought down, by something else between preview and confirmation.
- A prune is issued while another process is mid-build, so the set of removable objects changes underneath it; the verification step must report the discrepancy.
- An image is referenced only by a stopped container — reclaimable, but not unreferenced, and the difference must be visible.
- A cluster node container is stopped externally while the operator is connected to that cluster.
- The runtime restarts, invalidating every open stream and session.
- A project is adopted, then its last container is removed and later recreated — the adoption must still apply.
- A project is released while one of its operations is still in flight.
- A local cluster's node containers belong to no project, yet the cluster itself is one Fathom manages; the adoption rule must not leave them unactionable.
- An object is referenced by both an adopted and an unadopted project.
- The operator needs a setting no form exposes; the feature must say so rather than appearing to be unable to do it at all.
- The feature is reached in hosted mode, where no local runtime exists and exposing the host's daemon would escalate privilege.

## Requirements *(mandatory)*

### Functional Requirements

**Detection and presence**

- **FR-001**: The system MUST detect whether a container runtime is installed and whether it is currently running, reusing the existing runtime probe rather than adding a second one, and MUST bound that probe with a timeout so a hung runtime never blocks startup.
- **FR-002**: The Docker section MUST be absent when no runtime is installed, and MUST be present when one is installed, including when it is installed but not running.
- **FR-003**: The system MUST distinguish, in what it tells the operator, between: no runtime installed, installed but stopped, running but unreachable by this account, and running and reachable. A message that conflates any of these with "nothing is running" MUST NOT be shown.
- **FR-004**: The system MUST name which runtime it detected and which daemon endpoint it is talking to.
- **FR-005**: The Docker section MUST NOT appear as an entry in the kubeconfig context switcher, MUST NOT be representable as a fleet member, and MUST NOT be assignable to a fleet group.

**Inventory**

- **FR-006**: The system MUST list containers with, for each: identity, name, image, state, health where the runtime reports it, published ports, created and started times, and owning project where one is declared.
- **FR-007**: The system MUST group containers by their declared owning project, and MUST place containers that declare none in a distinct group that says so.
- **FR-008**: Each project group MUST state its total member count and how many members are running.
- **FR-009**: The system MUST list images, volumes and networks, each with its size or reclaimable space where the runtime reports it, and with the objects that currently reference it.
- **FR-010**: The system MUST distinguish tagged from untagged images everywhere either can be removed.
- **FR-011**: Inventory MUST be identified by runtime object identity, and every operation MUST target that identity rather than a display name.

**Adoption: which projects Fathom may act on**

- **FR-012**: Every project, and the group of containers belonging to no project, MUST be either adopted or unadopted, and MUST state which it is.
- **FR-013**: An unadopted project MUST be fully readable — its containers, states, ports, images, references, sizes, log output and resource usage — and MUST NOT offer any mutating operation and MUST NOT offer an interactive session, because a session inside a container is unbounded execution in someone else's project.
- **FR-014**: The operator MUST be able to adopt a project explicitly, and to release an adopted project afterwards; releasing MUST return it to read-only.
- **FR-015**: Adoption MUST be recorded per project rather than per container, MUST persist across restarts, and MUST apply to containers that join an adopted project after it was adopted.
- **FR-016**: A local cluster that Fathom manages, and the containers backing it, MUST be actionable without a separate adoption step, and the cross-link in FR-039 and FR-040 MUST NOT be weakened by the adoption rule.
- **FR-017**: Wherever an operation is unavailable because its project is unadopted, the system MUST state that as the reason and MUST offer adoption. A control MUST NOT be silently absent.

**Mutation: shared discipline**

- **FR-018**: Every mutating operation MUST show, before it is issued, the exact operation and the exact objects it will act on, in a form the operator could reproduce themselves.
- **FR-019**: Every operation this feature can issue MUST be describable in advance. The system MUST NOT offer a free-text command field or any other path to an operation whose effect cannot be previewed and whose objects cannot be enumerated.
- **FR-020**: No mutating operation MUST be issued without explicit operator confirmation, and the system MUST NOT reflect a changed state until the runtime reports the result.
- **FR-021**: Failures MUST be reported with the runtime's own message verbatim.
- **FR-022**: An operation affecting several objects MUST report a per-object outcome and MUST NOT report overall success when any object failed.
- **FR-023**: Destructive operations — any removal, any prune, and bringing a project down — MUST require a typed confirmation, MUST show the full list of objects to be removed with nothing reduced to a bare count, and MUST be verified afterwards against what was actually removed, with any discrepancy reported.

**Blast radius on a shared daemon**

- **FR-024**: Every destructive preview MUST name which projects fall inside its blast radius, including projects other than the one the operator is working in.
- **FR-025**: A destructive operation MUST NOT remove an object belonging to an unadopted project. Where such an object falls inside a blast radius, the operation MUST exclude it, and the preview MUST say it was excluded and which project it belongs to.
- **FR-026**: A prune or removal whose effect extends beyond the selected project MUST say so explicitly, naming those projects, before it can be confirmed.
- **FR-027**: The system MUST NOT offer any destructive operation whose scope cannot be enumerated in advance.

**Mutation: operations**

- **FR-028**: Operators MUST be able to start, stop, restart, pause, unpause and kill an individual container in an adopted project.
- **FR-029**: Operators MUST be able to apply those lifecycle operations to every container in one adopted project as a single action with a single preview.
- **FR-030**: Operators MUST be able to remove containers, images, volumes and networks, and to prune each of those classes, subject to FR-025.
- **FR-031**: Operators MUST be able to create and start a container, build an image from a context, and create a network and a volume, each from a form over that object's documented fields.
- **FR-032**: Operators MUST be able to bring an adopted project up, down, and restart it, acting on the project's own definition file.
- **FR-033**: When a project's definition file is absent or unreadable, project-level operations MUST be unavailable for that project with the reason stated, and the file MUST NOT be rewritten, moved or deleted.
- **FR-034**: Where a setting is not exposed by any form, the system MUST say plainly that the case is not covered, rather than implying the operation is impossible.

**Logs, sessions and usage**

- **FR-035**: Operators MUST be able to follow any container's log output live, adopted or not, with the same bounding, explicit truncation marking and search behaviour as cluster log views.
- **FR-036**: Operators MUST be able to open an interactive session inside a running container in an adopted project, and closing the view MUST end the session leaving no process behind.
- **FR-037**: The system MUST report current CPU and memory per container and an aggregate per project, for adopted and unadopted projects alike, and MUST show an unavailable figure as unavailable with its reason rather than as zero.
- **FR-038**: Usage history MUST be bounded with an explicit cap, and reaching that cap MUST be visible.

**Cluster cross-link**

- **FR-039**: A container that the runtime identifies as a node of a local cluster MUST state which cluster, and MUST link to that cluster's views.
- **FR-040**: A local cluster MUST list the containers backing it, with their current state, linking back to the Docker section.
- **FR-041**: A preview of stopping or removing a container that backs a cluster MUST warn that the cluster will break and name it, and MUST additionally state when the operator is currently connected to that cluster.
- **FR-042**: A container that carries no cluster identity MUST NOT have a cluster inferred for it.

**Hosted mode, contract and quietness**

- **FR-043**: Every command in this feature MUST be denied in hosted mode, and the denial MUST state the reason: a server has no local container runtime, and exposing the host's daemon to signed-in users would grant them privileges they do not have.
- **FR-044**: All runtime access MUST live in the backend, with the protocol defined once and client types generated from it; drift MUST fail the build or tests.
- **FR-045**: The system MUST NOT poll or sample the runtime while the Docker section is not being shown, and MUST NOT measurably increase idle resource use.
- **FR-046**: Streams and sessions belonging to a view the operator has left MUST be closed, and events from a superseded request MUST be discarded rather than rendered.
- **FR-047**: An unreadable or invalid project definition file, unreadable runtime metadata, or an unreadable record of which projects are adopted MUST fall back to a usable state without deleting or modifying the operator's files. A lost adoption record MUST fall back to everything unadopted, never to everything adopted.

### Key Entities

- **Runtime**: the detected container runtime — which one, whether installed, whether running, which endpoint, and whether this account can reach it.
- **Container**: one container as the runtime reports it — identity, name, image, state, health, ports, times, owning project, and cluster identity when it is a cluster node.
- **Project**: a named group of containers declared by the containers themselves, with its definition file when one is known, its member count, its running count, and whether it is adopted.
- **Adoption**: the operator's recorded declaration that Fathom may act on one project, persisted across restarts and revocable. Its absence is the default.
- **Image / Volume / Network**: a removable runtime object with its size or reclaimable space, whether it is tagged (images), and the objects currently referencing it.
- **Usage sample**: CPU and memory for one container at one instant, aggregated per project, retained in a bounded history.
- **Operation preview**: the exact operation about to be issued, the objects it targets, and — for destructive operations — the full removal list, the space it frees, the projects in its blast radius, any object excluded for belonging to an unadopted project, and any cluster it would break.
- **Operation outcome**: what the runtime actually did, per object, including verbatim failures and any discrepancy against the preview.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a machine with no container runtime, the Docker section does not appear; on a machine where the runtime is installed but stopped, the operator is told exactly that, naming the runtime, in 100% of cases. "No containers" is never shown for a runtime that is absent, stopped or unreachable.
- **SC-002**: Given a daemon shared by four unrelated projects, an operator can state which containers belong to which project without opening any individual container, on first viewing.
- **SC-003**: Zero mutating operations can be issued against a project the operator has not adopted, and 100% of the controls that are unavailable for that reason say so and offer adoption.
- **SC-004**: Every project is readable — containers, states, logs and usage — without adopting it, so read-only is the default without being a reduced view.
- **SC-005**: 100% of destructive operations enumerate every object they will remove before they can be confirmed; no destructive operation can be issued from a summary count alone.
- **SC-006**: 100% of destructive previews whose effect reaches a project other than the selected one name those projects before confirmation, and any object excluded for belonging to an unadopted project is named as excluded.
- **SC-007**: Every operation reachable from this feature can be described in advance; there is no path from the interface to an operation whose objects are not enumerable. Verified by inspection of the available operations, with zero exceptions.
- **SC-008**: 100% of previews for stopping or removing a container that backs a local cluster name that cluster, and say when the operator is connected to it.
- **SC-009**: Restarting a known stuck service in an adopted project takes under 15 seconds and no terminal, from the Docker section being open.
- **SC-010**: The operator can answer "what is safe to delete, and how much does it free" for images, volumes and networks without issuing any operation, and the space actually reclaimed matches the predicted figure to within the runtime's own reporting granularity.
- **SC-011**: With the Docker section closed, the app's idle resource use is indistinguishable from before this feature, and stays under the existing idle budget of 2% of one core.
- **SC-012**: With 500 containers present, operator input in the Docker section is handled in under 100 ms.
- **SC-013**: A corrupt or unreadable project definition file leaves that file byte-identical on disk, and the Docker section stays usable; a lost adoption record results in nothing being actionable rather than everything.
- **SC-014**: Every command in this feature is denied in hosted mode, with a reason that states why rather than only that it was refused.
- **SC-015**: After any destructive operation, the operator is shown what was actually removed, and a mismatch against the preview is surfaced rather than ignored.

## Assumptions

- Creation is form-driven over documented object fields only. There is deliberately no free-text command field: such a field is arbitrary command execution launched from the interface, its objects cannot be enumerated in advance, and it would make the preview guarantee in FR-018 unkeepable. Flags and combinations no form covers are reached from a terminal, and the feature says so rather than pretending to cover them.
- "Full parity" accordingly means the object operations an operator performs on containers, images, volumes, networks and projects — lifecycle, removal, prune, creation, build, and project up/down/restart — not every flag of the runtime's command-line interface.
- Read-only is the default for every project, and mutability is something the operator grants per project. This is a deliberate cost: stopping one container in an unrelated project requires adopting that project first. The daemon on the target machine is shared by several unrelated projects and a careless destructive action has already cost real work, so an action that cannot reach an unadopted project is worth the extra step.
- Local clusters Fathom manages are actionable without adoption, because the operator already created them through Fathom and the cluster views already guard those operations.
- Docker-compatible alternative runtimes are in scope, because the existing probe already detects them and their socket API is compatible. Object identity, labels, sizes and references are read from whichever runtime answered, never assumed from one implementation's behaviour.
- If the environment points at a daemon that is not on this machine, the feature still works and names which daemon it reached, so "local" is never implied falsely. Hosted mode is denied regardless, because the objection there is privilege, not locality.
- A container's owning project and a cluster node's cluster identity are read from the labels the respective tools already write. A container without those labels has no project and no cluster — nothing is inferred from naming patterns.
- An account that cannot open the runtime socket is treated as a reportable condition with a remediation, not as an absent runtime.
- Log streaming, bounded buffering, truncation marking and the interactive terminal are reused from the existing cluster log and terminal implementations rather than built again. Usage sampling reuses the existing bounded-history approach.
- The typed-confirmation, preview and post-action verification guard is the one already established for destructive cluster actions, applied to runtime objects.
- This feature manages a runtime, not a cluster. It introduces no new cluster access, and adds no API server requests.
- Specification only: no plan, no tasks and no implementation are produced by this document.

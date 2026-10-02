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

The operator has a stuck service. From the Docker section they start, stop, restart, pause, unpause
or kill a single container, or do the same to every container in one project at once. Before
anything runs, Fathom shows the exact operation it is about to issue, against the exact objects,
and nothing happens until the operator confirms. The result is reported from what the runtime
actually did, not assumed.

**Why this priority**: It is the daily-workflow win and the reason the feature was asked for. On
its own, with only US1 beneath it, it already replaces the common trip to a terminal.

**Independent Test**: Stop a single running container and confirm the preview named that container
and that operation, the container is stopped afterwards, and the row's state came from the runtime
rather than being set optimistically. Then restart a whole project and confirm every member was
named in the preview.

**Acceptance Scenarios**:

1. **Given** a running container, **When** the operator chooses Stop, **Then** the exact operation and the container it targets are shown before it is issued, and the operator must confirm.
2. **Given** the operator confirms, **When** the runtime reports the result, **Then** the row's new state reflects what the runtime reported, and a failure is shown with the runtime's own message verbatim.
3. **Given** a project with four containers, **When** the operator restarts the project, **Then** all four are named in the preview before anything runs, and a per-container outcome is reported for each.
4. **Given** a project-level operation where one container fails, **When** the operation finishes, **Then** the outcome states which succeeded and which failed, and does not report overall success.
5. **Given** a container is removed outside Fathom between the preview and the confirmation, **When** the operation is issued, **Then** the runtime's "no such container" is reported verbatim and no other object is touched.

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
or a volume, and brings a whole project up or down from its own definition file — each one showing
the exact operation before it runs.

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

**Mutation: shared discipline**

- **FR-012**: Every mutating operation MUST show, before it is issued, the exact operation and the exact objects it will act on, in a form the operator could reproduce themselves.
- **FR-013**: No mutating operation MUST be issued without explicit operator confirmation, and the system MUST NOT reflect a changed state until the runtime reports the result.
- **FR-014**: Failures MUST be reported with the runtime's own message verbatim.
- **FR-015**: An operation affecting several objects MUST report a per-object outcome and MUST NOT report overall success when any object failed.
- **FR-016**: Destructive operations — any removal, any prune, and bringing a project down — MUST require a typed confirmation, MUST show the full list of objects to be removed with nothing reduced to a bare count, and MUST be verified afterwards against what was actually removed, with any discrepancy reported.

**Mutation: blast radius on a shared daemon**

- **FR-017**: Every destructive preview MUST name which projects fall inside its blast radius, including projects other than the one the operator is working in.
- **FR-018**: A prune or removal whose effect extends beyond the selected project MUST say so explicitly, naming those projects, before it can be confirmed.
- **FR-019**: The system MUST NOT offer any destructive operation whose scope cannot be enumerated in advance.

**Mutation: operations**

- **FR-020**: Operators MUST be able to start, stop, restart, pause, unpause and kill an individual container.
- **FR-021**: Operators MUST be able to apply those lifecycle operations to every container in one project as a single action with a single preview.
- **FR-022**: Operators MUST be able to remove containers, images, volumes and networks, and to prune each of those classes.
- **FR-023**: Operators MUST be able to create and start a container, build an image from a context, and create a network and a volume.
- **FR-024**: Operators MUST be able to bring a project up, down, and restart it, acting on the project's own definition file.
- **FR-025**: When a project's definition file is absent or unreadable, project-level operations MUST be unavailable for that project with the reason stated, and the file MUST NOT be rewritten, moved or deleted. [NEEDS CLARIFICATION: see Q1 — whether creation is form-driven over documented object fields only, or includes a free-text command field]
- **FR-026**: [NEEDS CLARIFICATION: see Q2 — whether containers belonging to projects other than those Fathom manages are read-only by default]

**Logs, sessions and usage**

- **FR-027**: Operators MUST be able to follow a container's log output live, with the same bounding, explicit truncation marking and search behaviour as cluster log views.
- **FR-028**: Operators MUST be able to open an interactive session inside a running container, and closing the view MUST end the session leaving no process behind.
- **FR-029**: The system MUST report current CPU and memory per container, and an aggregate per project, and MUST show an unavailable figure as unavailable with its reason rather than as zero.
- **FR-030**: Usage history MUST be bounded with an explicit cap, and reaching that cap MUST be visible.

**Cluster cross-link**

- **FR-031**: A container that the runtime identifies as a node of a local cluster MUST state which cluster, and MUST link to that cluster's views.
- **FR-032**: A local cluster MUST list the containers backing it, with their current state, linking back to the Docker section.
- **FR-033**: A preview of stopping or removing a container that backs a cluster MUST warn that the cluster will break and name it, and MUST additionally state when the operator is currently connected to that cluster.
- **FR-034**: A container that carries no cluster identity MUST NOT have a cluster inferred for it.

**Hosted mode, contract and quietness**

- **FR-035**: Every command in this feature MUST be denied in hosted mode, and the denial MUST state the reason: a server has no local container runtime, and exposing the host's daemon to signed-in users would grant them privileges they do not have.
- **FR-036**: All runtime access MUST live in the backend, with the protocol defined once and client types generated from it; drift MUST fail the build or tests.
- **FR-037**: The system MUST NOT poll or sample the runtime while the Docker section is not being shown, and MUST NOT measurably increase idle resource use.
- **FR-038**: Streams and sessions belonging to a view the operator has left MUST be closed, and events from a superseded request MUST be discarded rather than rendered.
- **FR-039**: An unreadable or invalid project definition file, or unreadable runtime metadata, MUST fall back to a usable empty state without deleting or modifying the operator's files.

### Key Entities

- **Runtime**: the detected container runtime — which one, whether installed, whether running, which endpoint, and whether this account can reach it.
- **Container**: one container as the runtime reports it — identity, name, image, state, health, ports, times, owning project, and cluster identity when it is a cluster node.
- **Project**: a named group of containers declared by the containers themselves, with its definition file when one is known, its member count and its running count.
- **Image / Volume / Network**: a removable runtime object with its size or reclaimable space, whether it is tagged (images), and the objects currently referencing it.
- **Usage sample**: CPU and memory for one container at one instant, aggregated per project, retained in a bounded history.
- **Operation preview**: the exact operation about to be issued, the objects it targets, and — for destructive operations — the full removal list, the space it frees, the projects in its blast radius, and any cluster it would break.
- **Operation outcome**: what the runtime actually did, per object, including verbatim failures and any discrepancy against the preview.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a machine with no container runtime, the Docker section does not appear; on a machine where the runtime is installed but stopped, the operator is told exactly that, naming the runtime, in 100% of cases. "No containers" is never shown for a runtime that is absent, stopped or unreachable.
- **SC-002**: Given a daemon shared by four unrelated projects, an operator can state which containers belong to which project without opening any individual container, on first viewing.
- **SC-003**: 100% of destructive operations enumerate every object they will remove before they can be confirmed; no destructive operation can be issued from a summary count alone.
- **SC-004**: 100% of destructive previews whose effect reaches a project other than the selected one name those projects before confirmation.
- **SC-005**: 100% of previews for stopping or removing a container that backs a local cluster name that cluster, and say when the operator is connected to it.
- **SC-006**: Restarting a known stuck service takes under 15 seconds and no terminal, from the Docker section being open.
- **SC-007**: The operator can answer "what is safe to delete, and how much does it free" for images, volumes and networks without issuing any operation, and the space actually reclaimed matches the predicted figure to within the runtime's own reporting granularity.
- **SC-008**: With the Docker section closed, the app's idle resource use is indistinguishable from before this feature, and stays under the existing idle budget of 2% of one core.
- **SC-009**: With 500 containers present, operator input in the Docker section is handled in under 100 ms.
- **SC-010**: A corrupt or unreadable project definition file leaves that file byte-identical on disk, and the Docker section stays usable.
- **SC-011**: Every command in this feature is denied in hosted mode, with a reason that states why rather than only that it was refused.
- **SC-012**: After any destructive operation, the operator is shown what was actually removed, and a mismatch against the preview is surfaced rather than ignored.

## Assumptions

- "Full parity" means the object operations an operator performs on containers, images, volumes, networks and projects — lifecycle, removal, prune, creation, build, and project up/down/restart. It does not mean reimplementing every flag of the runtime's command-line interface; flags without a documented object-level meaning are out of scope, and Q1 below settles whether a free-text escape hatch exists at all.
- Docker-compatible alternative runtimes are in scope, because the existing probe already detects them and their socket API is compatible. Object identity, labels, sizes and references are read from whichever runtime answered, never assumed from one implementation's behaviour.
- If the environment points at a daemon that is not on this machine, the feature still works and names which daemon it reached, so "local" is never implied falsely. Hosted mode is denied regardless, because the objection there is privilege, not locality.
- A container's owning project and a cluster node's cluster identity are read from the labels the respective tools already write. A container without those labels has no project and no cluster — nothing is inferred from naming patterns.
- An account that cannot open the runtime socket is treated as a reportable condition with a remediation, not as an absent runtime.
- Log streaming, bounded buffering, truncation marking and the interactive terminal are reused from the existing cluster log and terminal implementations rather than built again. Usage sampling reuses the existing bounded-history approach.
- The typed-confirmation, preview and post-action verification guard is the one already established for destructive cluster actions, applied to runtime objects.
- This feature manages a runtime, not a cluster. It introduces no new cluster access, and adds no API server requests.
- Specification only: no plan, no tasks and no implementation are produced by this document.

## Outstanding Questions

### Q1: Does creation include a free-text command field?

**Context**: FR-023 and FR-025 — "Operators MUST be able to create and start a container, build an
image from a context, and create a network and a volume."

**What we need to know**: Whether creating and running is driven by forms over documented object
fields only, or whether there is also a free-text field where an arbitrary runtime command can be
typed and executed.

| Option | Answer | Implications |
|--------|--------|--------------|
| A      | Form-driven only, over documented object fields | Every operation is enumerable, previewable and auditable, which is what makes FR-012 and FR-016 possible. Some rarely used flags become unreachable, and the operator drops to a terminal for those. |
| B      | Forms, plus a free-text command field | True parity, including flags no form covers. But a free-text field is arbitrary command execution launched from the UI: its blast radius cannot be enumerated in advance, so it cannot satisfy FR-012 or FR-019, and it would need its own carve-out and its own warning. |
| C      | Forms now, free-text deferred to a later feature | Keeps this feature's guarantees intact and leaves the escape hatch as a deliberate, separately specified decision rather than a side effect. |
| Custom | Provide your own answer | For example: free-text allowed but read-only operations only, or allowed with mandatory confirmation of the literal command string. |

**Your choice**: _[awaiting answer]_

### Q2: Are other projects' containers read-only by default?

**Context**: FR-026, and the shared-daemon requirement behind FR-017 and FR-018. This daemon runs
several unrelated projects at once, and a careless destructive action has already cost real work.

**What we need to know**: Whether Fathom may mutate containers belonging to any project it finds,
or only to projects the operator has explicitly adopted, with everything else visible but read-only.

| Option | Answer | Implications |
|--------|--------|--------------|
| A      | Everything is mutable; blast-radius naming is the only protection | Simplest and most flexible — the operator can stop any container from one place. Protection rests entirely on reading the preview. |
| B      | Read-only unless the project is explicitly adopted | A mis-click cannot reach an unrelated project at all. Costs an adopt step per project, and an operator who just wants to stop one foreign container has to adopt its project first. |
| C      | Mutable, but foreign projects require the typed confirmation even for non-destructive lifecycle actions | Keeps one place to act from, while making a foreign action deliberate. Heavier than it needs to be for your own projects, lighter than adoption for the rest. |
| Custom | Provide your own answer | For example: adoption required only for destructive operations, with lifecycle actions open everywhere. |

**Your choice**: _[awaiting answer]_

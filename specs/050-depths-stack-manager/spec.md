# Feature Specification: Depths, a self-hosted Compose stack manager

**Feature Branch**: `050-depths-stack-manager`

**Created**: 2026-10-23

**Status**: Draft

**Input**: User description: "theres an app called dockage and i use it but i dislike it can we add to fathoms roster a docker / compose management tool compatible with dockage layour but with nice oidc / oauth support multi contianer tabs the works ? maybe since we have fathom we call it depths? it should be a docker container with a react tsx frontend same themeing and a axum tower backend" — then: "a ./crates with workspace to share context and concepts, than for anything pnpm related that needs to be shared to it under ./packages and we will rebrand the readme to be our orchestration companion suite". ("dockage" is Dockge.)

## Clarifications

### Session 2026-10-23

- Q: Where does Depths live? → A: In this repository, as a second app beside Fathom. Fathom is the Kubernetes companion, Depths the Docker-host companion, and the repository README presents both as one orchestration companion suite.
- Q: Copy Fathom's container and sign-in code, or share it? → A: Share it. What both apps need (container-runtime handling, sign-in, sessions, audit, theme and container UI) is extracted into shared building blocks that both apps consume. Fathom keeps working, and passing its tests, at every step of the extraction.
- Q: How does Depths relate to an existing Dockge install? → A: It reads and writes the same stacks directory in the same layout, so an operator can point Depths at a directory Dockge has been managing, use it, and go back to Dockge without converting anything.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Sign in and see every stack on the host (Priority: P1)

An operator runs Depths as one container on a Docker host, pointed at the stacks directory Dockge
has been using. They open it in a browser and are sent to their organisation's identity provider.
After signing in they see every stack in that directory: its name, whether it is running, partly
running or stopped, how many of its services are up, and when it last changed. Opening a stack shows
its compose file, its environment file and its containers, each with state, image, ports, uptime and
health. Containers on the host that belong to no stack in the directory sit in their own clearly named
group instead of being hidden or mixed in.

**Why this priority**: It is the smallest useful Depths, and it proves the two claims everything else
depends on: nobody reaches the host without signing in, and an existing Dockge directory is read
correctly as it is.

**Independent Test**: With a stacks directory holding three stacks (one running, one partly running,
one stopped) and two containers that belong to no stack, sign in and confirm three stacks appear with
the right status and service counts, each stack's files and containers match the disk and the runtime,
and the two loose containers appear in the "not in a stack" group.

**Acceptance Scenarios**:

1. **Given** a visitor who is not signed in, **When** they open any Depths page or call any Depths endpoint, **Then** they are sent to sign in (pages) or refused (endpoints), and no stack, container or host detail is disclosed.
2. **Given** a stacks directory last managed by Dockge, **When** Depths starts against it, **Then** every stack Dockge showed is listed under the same name, and no file in the directory is created, renamed or rewritten by starting up.
3. **Given** a stack directory whose compose file is named `compose.yaml`, `compose.yml`, `docker-compose.yml` or `docker-compose.yaml`, **When** the list loads, **Then** the stack is recognised whichever of those names it uses.
4. **Given** a sub-directory of the stacks directory with no compose file, **When** the list loads, **Then** it is not shown as a stack.
5. **Given** a stack whose compose file does not parse, **When** the list loads, **Then** the stack is still listed, marked invalid with the parser's message verbatim, and the rest of the list is unaffected.
6. **Given** the runtime becomes unreachable while the list is open, **When** the next refresh occurs, **Then** Depths says so with the reason verbatim and keeps the last known state marked as stale rather than blanking it.
7. **Given** a stack that was started from a terminal rather than from Depths, **When** the list loads, **Then** its status reflects its real containers.

---

### User Story 2 - Start, stop and update a stack safely (Priority: P1)

The operator brings a stack up, takes it down, restarts it, stops it or pulls newer images for it.
Before anything runs, Depths shows the exact command it is about to run, against which stack, and
what it will affect (which services and containers will be created, recreated, stopped or removed).
Nothing happens until they confirm. While the operation runs its output streams live. Afterwards the
stack's status is the status the runtime reports, not an assumed one, and the operation is in the
audit log under their name.

**Why this priority**: Starting and stopping stacks is why anyone runs a stack manager. Together with
US1 it is a full replacement for Dockge's daily use.

**Independent Test**: Bring a stopped stack up from Depths, confirm the preview names the stack, its
services and the command, confirm the output streamed and the stack is reported running, and confirm
one audit entry names the operator, the stack, the command and the outcome. Repeat for down, restart,
stop and pull.

**Acceptance Scenarios**:

1. **Given** a stopped stack, **When** the operator chooses "Up", **Then** a preview shows the exact command line, the stack directory it runs in, and the services that will start, and the command runs only after confirmation.
2. **Given** the operator dismisses the preview, **When** they look at the stack, **Then** nothing changed and no audit entry claims an operation ran.
3. **Given** an operation that fails part-way, **When** it finishes, **Then** the runtime's error is shown verbatim, the stack shows its real resulting state, and the audit entry records a failure.
4. **Given** "Down" on a stack with named volumes, **When** the preview is shown, **Then** it states whether volumes will be kept or removed, and keeping them is the default.
5. **Given** two operators act on the same stack at the same moment, **When** the second confirms, **Then** Depths refuses it with a message naming the operation already in progress, rather than running both.
6. **Given** a pull finishes with newer images, **When** the operator looks at the stack, **Then** it indicates that running containers use older images until the stack is recreated.

---

### User Story 3 - Edit, create and delete stacks (Priority: P2)

The operator edits a stack's compose file and environment file in the browser. The editor highlights
the syntax and flags invalid YAML and compose errors before saving. Saving shows a diff of the file
against what is on disk and writes only after confirmation. "Save and deploy" shows the diff and the
command together. They create a new stack by naming it and writing its compose file, and delete a stack
after a confirmation that states whether its containers are stopped first and whether its directory is
removed. Values in the environment file that look like secrets are masked until the operator reveals
them.

**Why this priority**: Editing in place is what turns Depths from a status page into the tool an
operator reaches for, but US1 and US2 already deliver value without it.

**Independent Test**: Edit a stack's compose file to add a service, confirm the diff shows exactly that
change, save, confirm the file on disk matches byte for byte apart from the change, deploy, and confirm
the new service is running. Create and delete a stack the same way.

**Acceptance Scenarios**:

1. **Given** an edit that makes the compose file invalid, **When** the operator tries to save, **Then** the error is shown at its line, and they can still save it explicitly if they choose (the file is theirs), with a warning that it will not deploy.
2. **Given** a compose file changed on disk by someone else since it was opened, **When** the operator saves, **Then** Depths refuses to overwrite it, shows the other change, and lets them reload or merge.
3. **Given** a new stack name that is not a valid compose project name, or already exists, **When** they try to create it, **Then** creation is refused with the reason.
4. **Given** "Delete" on a running stack, **When** the preview is shown, **Then** it lists the containers that will be stopped and removed and whether the stack directory will be deleted, and keeping the directory is the default.
5. **Given** an environment file with a `PASSWORD`, `SECRET`, `TOKEN` or `KEY` entry, **When** it is displayed, **Then** the value is masked until revealed, and revealed values never appear in logs or the audit log.
6. **Given** comments, ordering and formatting in a compose file, **When** the operator edits one line and saves, **Then** the rest of the file is preserved exactly.

---

### User Story 4 - Work with several containers side by side (Priority: P2)

Debugging a stack means watching more than one container. The operator opens a container's logs, an
interactive shell in another, and live CPU and memory for a third, each in its own tab, and arranges
tabs side by side. Tabs survive switching between stacks and a page reload. Logs follow live, can be
paused, searched and filtered by time, and keep a bounded history. The shell behaves like a terminal:
colours, resizing and control keys work.

**Why this priority**: It is the experience the operator asked for by name ("multi container tabs"),
and the thing Dockge does least well, but it sits on top of US1.

**Independent Test**: Open logs for container A, a shell in container B and stats for container C as
three tabs, arrange two side by side, reload the page, and confirm all three come back. Type a command
in B's shell and confirm its output. Confirm A's logs follow new lines and can be paused.

**Acceptance Scenarios**:

1. **Given** five open tabs, **When** the operator reloads the page, **Then** the same tabs, order and layout return, and streams reconnect.
2. **Given** a container that restarts while its log tab is open, **When** it comes back, **Then** the tab says it restarted and continues with the new container's output.
3. **Given** a container that stops while its shell tab is open, **When** the shell ends, **Then** the tab says the session ended and why, and offers to reconnect.
4. **Given** a very chatty container, **When** its logs stream for an hour, **Then** the tab stays responsive and holds a bounded history with a marker where older lines were dropped.
5. **Given** a narrow (phone) screen, **When** several tabs are open, **Then** one tab is shown at a time with a tab switcher, and nothing scrolls sideways.

---

### User Story 5 - Control who can do what, and see what was done (Priority: P2)

The owner, named in Depths' settings by their provider account id, is always an administrator.
Anyone else who signs in is recorded as pending and told they don't currently have access, until an
administrator lists them as a user or an administrator, or pre-adds them by account id. A user is
assigned specific stacks and sees only those: on them they can view files (with secrets masked, and
reveal them), logs and stats, run lifecycle operations, open shells and edit. An administrator sees
everything, can also create and delete stacks, remove volumes or images, see projects and containers
outside the stacks directory, manage the users list and read the full audit log. Anything Depths does
to the host or the users list is recorded with who, what, when, which stack and the outcome, and the
record cannot be edited or deleted from the UI.

**Why this priority**: Depths holds the keys to the host. Single shared passwords are the weakness
that prompted this feature, so per-person, per-stack access is core, but the first two stories already
require sign-in and work with the owner alone.

**Independent Test**: Start with only an owner configured. Sign in as person A and confirm the
no-access page; as the owner, confirm A is listed as pending. Make A a user on stack "media" only.
Without A signing in again, confirm A's next action works on "media" and that no other stack, foreign
project or loose container is visible to A. Remove "media" from A and confirm A's open session ends.
Confirm every change to A and every action A took is in the audit log.

**Acceptance Scenarios**:

1. **Given** a signed-in person who is not listed as a user or administrator, **When** they open Depths, **Then** they see a page saying they don't currently have access and who to ask, nothing about the host, and administrators see them in the pending list.
2. **Given** a user assigned stack "media", **When** they send a command for another stack directly, **Then** it is refused, the refusal is audited, and nothing on the host changes.
3. **Given** an administrator changes a user's role or stacks, **When** that user sends their next command, **Then** it is authorised against the new entry without restarting Depths or signing in again; removing the user or their last stack ends their sessions.
4. **Given** the audit log, **When** an administrator filters by person, stack or time, **Then** matching entries are shown, newest first, and can be exported.
5. **Given** a request that arrives from another site carrying a user's session, **When** it tries to mutate, **Then** it is refused.
6. **Given** the owner, **When** any administrator tries to demote or remove them, **Then** it is refused and audited.
7. **Given** no owner configured and no administrator in the users list, **When** Depths starts, **Then** it refuses with a message naming the owner setting.

---

### User Story 6 - Deploy Depths as one container, least privilege by default (Priority: P3)

The operator deploys Depths from a published container image and an example compose file. Setup needs
the stacks directory, access to the Docker runtime, and the identity provider's details. The example
shows running behind a socket proxy that exposes only what Depths needs, and documents what reaching
the Docker socket directly implies. Depths runs as a non-root user where the host allows it, and starts
with a clear error if any required setting is missing.

**Why this priority**: Required for anyone else to use Depths, but the earlier stories can be built and
tested before packaging is polished.

**Independent Test**: On a clean host, follow the guide with the example compose file, sign in, and
reach US1's list. Then repeat behind the socket proxy and confirm US1 and US2 still work.

**Acceptance Scenarios**:

1. **Given** a missing identity-provider setting, **When** Depths starts, **Then** it exits with a message naming the missing setting, and never starts without sign-in.
2. **Given** the example compose file, **When** it is brought up, **Then** Depths does not restart itself automatically unless the operator opts in.
3. **Given** a socket proxy that denies an operation Depths needs, **When** the operator tries it, **Then** the proxy's refusal is shown verbatim with a note pointing at the proxy configuration.

---

### User Story 7 - Find both apps as one suite (Priority: P3)

Someone landing on the repository sees one orchestration companion suite: Fathom for Kubernetes
clusters and Depths for Docker hosts, with what each is for, how to get each, and where each one's
guides are. Fathom users find nothing missing from what the README told them before.

**Why this priority**: It matters for discovery once Depths exists, but changes nothing for operators.

**Independent Test**: Read the README cold and confirm it names both apps, says which to choose for
which job, and links to each one's install steps and guides, and that every link resolves.

**Acceptance Scenarios**:

1. **Given** the README, **When** read top to bottom, **Then** both apps are introduced before installation details, and every link resolves.

### Edge Cases

- A stack directory or file the Depths process cannot read or write: it is listed, marked with the permission error, and offers no edit.
- A stack name containing characters the runtime rejects in project names: shown, marked invalid, never acted on.
- Symlinks in the stacks directory: followed only if they resolve inside the stacks directory; anything pointing outside is refused.
- A path in an edit or create request that tries to escape the stacks directory (`..`, absolute paths): refused and audited.
- A stack running under a different project name than its directory (started by hand with `-p`): shown as running elsewhere, not claimed as the stack.
- Dockge running at the same time against the same directory: allowed, because the files are the only state. An edit made by either is seen by the other on its next read.
- The identity provider is unreachable: sign-in fails with the provider's error. Existing sessions continue until they expire.
- A session expires during an open shell or log stream: the stream ends, the tab says so, and re-authentication restores it.
- Very large compose files or env files: there is a stated size limit beyond which the editor refuses to load the file and says why.
- Thousands of containers on the host: the list stays usable (see SC-004).
- Files Depths does not recognise inside a stack directory (build contexts, config files, `global.env`-style files): left untouched.

## Requirements *(mandatory)*

### Functional Requirements

**Stacks directory and layout**

- **FR-001**: Depths MUST treat a configured stacks directory as the only source of truth for stacks. A stack is a direct sub-directory containing a compose file named `compose.yaml`, `compose.yml`, `docker-compose.yml` or `docker-compose.yaml`, with an optional `.env` beside it. Depths MUST keep no other record of stacks.
- **FR-002**: The stack's name MUST be its directory name, and lifecycle operations MUST address the stack by that name as its compose project, from inside that directory, so a stack Depths starts is the same project Dockge or a terminal would see.
- **FR-003**: Depths MUST NOT create, rename, reformat or delete any file in the stacks directory except as the direct result of a confirmed operator action that named it.
- **FR-004**: When several compose file names exist in one stack directory, Depths MUST pick the same one the compose tool would, and say which file it is showing.
- **FR-005**: Depths MUST refuse any read or write that resolves outside the stacks directory.

**Viewing**

- **FR-006**: Depths MUST list every stack with its status (running, partly running, stopped, invalid), running and total service counts, and last modification time.
- **FR-007**: Depths MUST show each stack's compose file, environment file and containers (state, image, ports, uptime, health).
- **FR-008**: Containers and compose projects on the host that do not belong to a stack in the stacks directory MUST be shown to administrators in a separate, clearly named group, read-only.
- **FR-009**: Status MUST come from the runtime. Depths MUST NOT show a state change before the runtime reports it.
- **FR-010**: When the runtime is unreachable, Depths MUST say so with the reason verbatim and keep the last known state marked stale.

**Lifecycle**

- **FR-011**: Operators MUST be able to run up, down, restart, stop and pull on a stack.
- **FR-012**: Every operation that changes the host MUST be preceded by a preview showing the exact command line, its working directory, the stack, and the services and containers it affects, and MUST run only after explicit confirmation.
- **FR-013**: Operation output MUST stream to the operator live, and failures MUST be shown verbatim.
- **FR-014**: Depths MUST run at most one mutating operation per stack at a time and refuse a concurrent one with a message naming the running one.
- **FR-015**: Destructive options (removing volumes, removing images, deleting the stack directory) MUST be off by default and named in the preview when on.

**Editing**

- **FR-016**: Users MUST be able to edit the compose file and environment file of a stack assigned to them. Administrators MUST be able to edit any stack, create a stack, and delete a stack.
- **FR-017**: The editor MUST highlight YAML, and report YAML and compose validation errors at their line before saving.
- **FR-018**: Saving MUST show a diff against the file on disk and write only after confirmation. Content the operator did not change MUST be preserved byte for byte.
- **FR-019**: Saving MUST be refused when the file changed on disk since it was loaded, showing the other change.
- **FR-020**: "Save and deploy" MUST show the diff and the deploy command in one preview.
- **FR-021**: Environment values whose key suggests a secret MUST be masked until revealed, and MUST NOT appear in logs or the audit log.
- **FR-022**: New stack names MUST be valid compose project names and unique. Invalid names MUST be refused with the reason.

**Containers and tabs**

- **FR-023**: Users and administrators MUST be able to open live logs, live CPU, memory, network and I/O stats, and an interactive shell for any container they may see.
- **FR-024**: Logs, shells and stats MUST open as tabs, several at once, arrangeable side by side at wide widths and one at a time at narrow widths.
- **FR-025**: Open tabs, their order and layout MUST survive navigation and page reload for that person.
- **FR-026**: Log tabs MUST follow live, support pause, text search and time filtering, and keep a bounded history with a visible marker where older lines were dropped.
- **FR-027**: Shell tabs MUST support colour, resizing and control keys, and MUST say when and why a session ends.

**Sign-in and access**

- **FR-028**: Every page and endpoint except sign-in, sign-in callback and health MUST require a signed-in session. There MUST be no anonymous mode and no shared password.
- **FR-029**: Depths MUST support sign-in through any standards-based OpenID Connect provider, and through OAuth 2.0 providers that are not OIDC (GitHub and Discord as the named cases) by reading the person's identity (an immutable account id, name and email) from the provider. Depths MUST NOT need team, guild, role or organisation scopes.
- **FR-030**: Administrators MUST be able to give a person the role user or administrator, assign a user a list of stacks, change or remove them, and pre-add a person by provider account id before they first sign in. A user MUST see and act on only their assigned stacks; creating and deleting stacks, removing volumes or images, viewing projects and containers outside the stacks directory, the audit log and the users list MUST be administrator-only.
- **FR-031**: A signed-in person who is not listed as a user or administrator MUST be recorded as pending, get no access, and see a page saying they don't currently have access. A user with no stacks gets the same page.
- **FR-032**: Every request MUST be authorised server-side against the person's current entry in the users list. Hiding a control in the UI MUST NOT be the only protection.
- **FR-033**: A change to a person's role or stacks MUST take effect at that person's next command, without restarting Depths or signing in again. Removing a person, or their last stack, MUST end their sessions. A provider token the provider rejects at refresh MUST end the session.
- **FR-034**: Mutating requests MUST be protected against cross-site request forgery.

**Audit**

- **FR-035**: Every mutation attempt, including refused and failed ones, MUST be recorded with the person's identity, time, stack, operation, exact command or file diff summary, and outcome.
- **FR-036**: The audit log MUST be append-only from Depths' side. Nothing in the UI or API can edit or delete an entry.
- **FR-037**: Administrators MUST be able to filter the audit log by person, stack and time, and export it.

**Deployment and security**

- **FR-038**: Depths MUST ship as a single container image whose settings (stacks directory, runtime address, identity provider, owner, location of the web assets) come from configuration at start-up.
- **FR-039**: Depths MUST start only when sign-in is fully configured, and exit with a message naming any missing setting.
- **FR-048**: Depths MUST take one or more owners from its settings as provider account ids (a single generic owner setting is accepted as an alias). Owners MUST always be administrators and MUST NOT be demotable or removable from the UI. Depths MUST refuse to start, naming the owner setting, when no owner is set and the users list holds no administrator. The users list MUST be Depths' own state, written only by Depths, atomically, readable only by Depths' user, and every change to it MUST be audited.
- **FR-047**: Depths MUST accept the settings a Dockge host's environment file already carries (the stacks directory under Dockge's name, and a single generic OAuth provider, client id, client secret and public origin), so it can be added beside Dockge on the same environment file without renaming anything. Depths' own setting names win when both are set.
- **FR-040**: Depths MUST work when its runtime access goes through a socket proxy, and the example deployment MUST show that setup.
- **FR-041**: The example deployment MUST NOT restart automatically unless the operator opts in.
- **FR-042**: Depths MUST run as a non-root user wherever runtime access allows it, and the guide MUST state that runtime access is equivalent to root on the host.

**Shared foundations and suite**

- **FR-043**: The container-runtime handling, sign-in, session, CSRF and audit capabilities that Depths shares with Fathom MUST be provided by shared building blocks used by both apps, not by copies.
- **FR-044**: Fathom's existing features, tests and release builds MUST keep working unchanged through the extraction.
- **FR-045**: Depths' interface MUST use the same visual theme as Fathom.
- **FR-046**: The repository README MUST present Fathom and Depths as one orchestration companion suite.

### Key Entities

- **Stacks directory**: The configured folder holding stacks. The only persistent record of stacks.
- **Stack**: A sub-directory with a compose file and optional environment file. Its name is its directory name and its compose project name. Has a derived status from its containers.
- **Loose container / foreign project**: Runtime objects not owned by any stack in the directory. Shown, never acted on.
- **Operation**: A requested lifecycle action or file change against one stack: who, what, the exact command or diff, status, output, outcome.
- **Person**: A signed-in identity from the provider, keyed by provider and immutable account id.
- **User entry**: A person's place in the users list: pending, user (with assigned stacks) or administrator, plus whether they are an owner, and when they were first and last seen.
- **Audit entry**: An append-only record of one operation attempt.
- **Tab**: An open log, shell or stats view of one container, with its position in the person's layout.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator with an existing Dockge stacks directory goes from a clean host to signing in and seeing all their stacks in under 10 minutes by following the guide.
- **SC-002**: Switching from Dockge to Depths and back changes no file in the stacks directory (verified by checksumming the directory before and after).
- **SC-003**: 100% of host-changing operations in the end-to-end suite are preceded by a preview and produce exactly one audit entry.
- **SC-004**: With 200 stacks and 2,000 containers, the stack list is usable within 2 seconds of sign-in and user input responds in under 100 ms.
- **SC-005**: Six tabs (logs, shells, stats) stay open and live for an hour with the interface still responding in under 100 ms.
- **SC-006**: No page or endpoint other than sign-in and health returns host information to an unauthenticated request (verified by automated sweep of every route).
- **SC-007**: Fathom's full test suite and release builds pass at every commit of the extraction.
- **SC-008**: An idle Depths with no open tabs uses under 2% of one CPU core.

## Assumptions

- Users are self-hosters and small teams running one Docker host each. Managing several hosts from one Depths (Dockge's "agents") is out of scope for v1.
- An identity provider is available (self-hosted OIDC such as Authelia, Authentik, Keycloak or Pocket ID, a cloud OIDC provider, GitHub, or Discord). Running without one is out of scope by design.
- The first real deployment is beside Dockge on the owner's game-server host, on its own port (8080), reached from the internet only through the owner's cluster gateway over the tailnet, never by publishing the home address.
- Editing compose files as free text is intended. Unlike Fathom's form-only container creation (spec 048), the file is the operator's own and the diff preview states its full effect.
- Stacks Depths did not create are fully manageable once they are in the stacks directory: putting a stack in that directory is the operator's declaration that Depths may manage it. Projects outside the directory are read-only (FR-008), the same protection 048's adoption gives.
- The audit log and the users list are Depths' own state and live outside the stacks directory, so they never disturb Dockge compatibility.
- Building images from a stack's build context is covered by "up" when the compose file asks for it. A separate build UI is out of scope for v1.
- Access is a users list kept by Depths, not groups read from the provider. A self-hosted Depths has a handful of people, the owner signs in with Discord or GitHub (whose guilds and teams would need extra scopes and rate-limited calls), and a list the owner edits in the UI is simpler to reason about. Granting by provider group is a later option (research R6).
- An owner given as a GitHub login is pinned to that account's numeric id at its first sign-in, because logins can be renamed and re-registered.
- An assignment to a stack that no longer exists is kept and shown as missing, so a stack deleted and recreated under the same name is still assigned. Creating a stack is administrator-only, so a user cannot claim a name.
- Admins can pre-add a person by provider account id. Invite links are out of scope for v1.
- Desktop and mobile browsers in their current versions are supported. Depths is a web app only; it has no desktop package.

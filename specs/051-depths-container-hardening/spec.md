# Feature Specification: Depths container hardening

**Feature Branch**: `050-depths-stack-manager` (spec 051 builds on 050 and lands on the same branch)

**Created**: 2026-10-24

**Status**: Draft

**Input**: User description: "if they have shell access thats the bugger concern, we want to hard prefer on the instance rootless containers for stacks and show a little warning if a container is root that'd be nice. And we want to hwrd guard about someone breaking containment on container shell escape"

## Clarifications

### Session 2026-10-24

- Q: Is revealing `.env` secrets to people with edit or shell rights a concern? → A: Not a major one. A person with a shell can read the environment anyway. The real risk is a shell that can break out of its container, because that is the host.
- Q: Should Depths change people's compose files to make them rootless? → A: No. It warns and explains, and an operator can choose to block the most dangerous configurations. It never rewrites a file.
- Q: Who may open a shell into a container running as root? → A: Under the default policy, administrators only. No policy ever lets a non-administrator into a container that can reach the host.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See which containers run as root or can reach the host (Priority: P1)

An operator browsing their stacks sees, at a glance, which containers are safe, which run as root,
and which are configured so that someone inside them could reach the host (privileged, holding the
Docker socket, sharing the host's process list, and so on). The marker is small and quiet for root
and unmistakable for host-reaching containers. Hovering, focusing or tapping it lists the reasons in
plain words. A stack shows the worst of its containers in the stack list. When the Docker daemon
itself runs as root, administrators see a notice linking to a guide on rootless operation.

**Why this priority**: Nobody can fix what they cannot see, and the shell guard (Story 2) is built on
the same rating.

**Independent Test**: With one container running as a non-root user, one running as root and one
privileged container with the Docker socket mounted, open the stack and confirm the three ratings
(ok, root, escape), the reasons listed for each, and the stack's worst rating in the list.

**Acceptance Scenarios**:

1. **Given** a container whose configured user is empty, `root` or uid 0 on a daemon that runs as root, **When** its stack is shown, **Then** the container carries a "root" marker whose reasons say it runs as root.
2. **Given** a container that is privileged, mounts a container-runtime socket or a sensitive host path, shares the host's process, IPC or user namespace, holds a capability that allows reaching the host, has host devices, or runs with its security profile turned off while root, **When** its stack is shown, **Then** it carries an "escape" marker listing every such reason.
3. **Given** a daemon running rootless or with user-namespace remapping, **When** a container runs as uid 0 with nothing else risky, **Then** it is rated ok, and the reason text explains that container root is not host root here.
4. **Given** a stack with one escape and two ok containers, **When** the stack list is shown, **Then** the stack row shows the escape marker.
5. **Given** an administrator on a rootful daemon, **When** they open the overview, **Then** a one-line notice links to the rootless guide; a user does not see it.

---

### User Story 2 - Shells cannot be used to break out of containment (Priority: P1)

A person opens a shell into a container. Depths checks the container's configuration at that moment,
not a cached copy. Under the default (strict) policy, nobody can open a shell into a container rated
escape, only administrators can open one into a container rated root, and everyone else follows their
normal role. The shell button shows why it is unavailable. The shell itself is started without any
extra privilege, and the person cannot choose to run it as another user or from another directory.
Every shell opened and every shell refused is recorded with the container's rating and reasons.

**Why this priority**: A shell in a container that can reach the host is root on the host. This is the
largest risk Depths carries.

**Independent Test**: As a user with shell rights on a stack, try to open shells into an ok, a root and
an escape container and confirm: ok opens; root and escape are refused with their reasons, and both
refusals are in the audit log. As an administrator, confirm root opens and escape is refused. Switch the
policy to relaxed and confirm the administrator can now open escape and the user still cannot.

**Acceptance Scenarios**:

1. **Given** the strict policy, **When** anyone asks for a shell into an escape container, **Then** it is refused, the reasons are shown, and the refusal is audited.
2. **Given** the strict policy, **When** a user asks for a shell into a root container, **Then** it is refused and audited; an administrator's request opens.
3. **Given** the relaxed policy, **When** an administrator asks for a shell into an escape container, **Then** it opens and is audited with the escape reasons; a user's request is still refused.
4. **Given** a container that was ok when the page loaded but was recreated as privileged since, **When** a shell is requested, **Then** the decision uses the container's current configuration and the shell is refused.
5. **Given** the container's configuration cannot be read at the moment of the request, **When** a shell is requested, **Then** it is refused (fail closed).
6. **Given** a crafted request that names a user, working directory, extra privilege or a shell path containing anything other than an absolute path, **When** it reaches Depths, **Then** the extra fields have no effect or the request is refused.

---

### User Story 3 - Prefer rootless stacks when editing and deploying (Priority: P2)

When someone edits a stack's compose file, or is about to bring it up, Depths lists warnings for each
service that has no non-root user, is privileged, adds dangerous capabilities, uses host namespaces or
devices, mounts sensitive host paths, or turns off its security profile. The warnings appear in the
editor and in the preview before saving or deploying. By default they only warn. An operator can turn on
an enforce policy, under which saving or deploying a file with escape-level findings is refused unless an
administrator explicitly acknowledges it, and the acknowledgement is audited. Root-only findings never
block.

**Why this priority**: Rating running containers (Story 1) catches problems after the fact; this catches
them before they run. It is advisory by default, so it is lower priority than the guard.

**Independent Test**: Edit a compose file to add `privileged: true` to one service and remove `user:` from
another. Confirm the editor and the preview show both warnings with the right levels. Under the warn policy
confirm saving works. Under enforce confirm a user cannot save or deploy, and an administrator can only
after ticking the acknowledgement, which then appears in the audit log.

**Acceptance Scenarios**:

1. **Given** a compose file with a service that has no `user:`, **When** it is edited or previewed, **Then** a root-level warning names that service.
2. **Given** a compose file with escape-level settings, **When** it is previewed under the warn policy, **Then** escape warnings are shown and saving or deploying is still possible.
3. **Given** the enforce policy, **When** a user saves or deploys a file with escape-level findings, **Then** it is refused with the findings, and audited.
4. **Given** the enforce policy, **When** an administrator saves or deploys such a file and acknowledges the findings, **Then** it proceeds and the acknowledgement and findings are in the audit log.
5. **Given** any policy, **When** a file has only root-level findings, **Then** nothing is blocked.
6. **Given** any policy, **When** warnings are shown, **Then** the compose file on disk is never changed by Depths except by the person's own save.

### Edge Cases

- A compose file that does not parse: lint findings are not shown; the existing parse error is.
- Variable interpolation (`${VAR}`) in a risky field: the finding is reported against the field as written, and noted as depending on the variable.
- A named user (`user: app`) is treated as non-root; Depths cannot resolve names inside the image and says so in the reason.
- A container rated escape is also rated root when it runs as uid 0; the higher level wins and both reasons are shown.
- Very many findings: the marker shows the level and a count; the list is complete in the detail.
- The daemon's mode cannot be determined: Depths assumes rootful (the safer reading).
- Read-only bind mounts of sensitive paths count as escape only for paths that reveal or control the host (runtime sockets always; `/` and `/etc` read-only count as root-level information disclosure, not escape; see FR-004).

## Requirements *(mandatory)*

### Functional Requirements

**Container ratings**

- **FR-001**: Depths MUST rate every container it shows as ok, root or escape, from its current runtime configuration, with one plain-language reason for each finding.
- **FR-002**: A container MUST be rated root when it runs as uid 0 (configured user empty, `root`, `0`, or a `0:`/`root:` user-and-group form) on a daemon that is not rootless and does not remap user namespaces.
- **FR-003**: A container MUST be rated escape when any of these hold: it is privileged; it mounts a container-runtime socket; it has a writable mount of `/`, `/etc`, `/root`, `/home`, `/boot`, `/dev`, `/proc`, `/sys`, `/run`, `/lib/modules` or the runtime's data directory; it shares the host's PID, IPC or user namespace; it holds SYS_ADMIN, SYS_MODULE, SYS_RAWIO, DAC_READ_SEARCH, BPF, or SYS_PTRACE together with the host PID namespace; it has host devices; or it runs as root with its seccomp or AppArmor profile turned off.
- **FR-004**: A read-only mount of `/` or `/etc` MUST be reported as a root-level finding; a runtime socket mounted read-only MUST still be escape.
- **FR-005**: Ratings MUST be shown on container rows, in the console's container picker and, as the worst of its containers, on each stack row, with the reasons reachable by hover, keyboard focus and touch.
- **FR-006**: Depths MUST detect whether the daemon runs rootless or with user-namespace remapping, treat an undetectable mode as rootful, and show administrators a notice with a link to the rootless guide when it is rootful.

**Shell guard**

- **FR-007**: Depths MUST support a shell policy setting with the values strict (default) and relaxed, read at start-up.
- **FR-008**: Under strict, a shell into an escape container MUST be refused for everyone, a shell into a root container MUST be refused for anyone but administrators, and a shell into an ok container MUST follow the person's role.
- **FR-009**: Under relaxed, a shell into an escape container MUST be refused for anyone but administrators, and a root container MUST follow the person's role. No setting MUST let a non-administrator open a shell into an escape container.
- **FR-010**: The shell decision MUST use the container's configuration read at the moment of the request, and MUST refuse if that read fails.
- **FR-011**: Depths MUST start shells with no added privilege, MUST NOT accept a user, working directory, environment or privilege from the request, and MUST accept only an absolute shell path made of path-safe characters.
- **FR-012**: Every shell opened and every shell refused MUST be audited with the person, container, stack, rating and reasons.
- **FR-013**: The UI MUST disable the shell control and show the reason when the policy would refuse it; the server's decision remains authoritative.
- **FR-014**: An open shell MUST end when its container is removed or recreated.

**Compose lint**

- **FR-015**: Depths MUST lint a stack's compose file for: a service with no non-root `user:`; `privileged`; dangerous added capabilities (as FR-003); host network, PID, IPC or user namespace modes; devices; sensitive host bind mounts (as FR-003/FR-004); and security options that turn confinement off. Each finding names the service, the field and the level.
- **FR-016**: Lint findings MUST be shown in the editor while editing and in the preview before saving or deploying.
- **FR-017**: Depths MUST support a compose policy setting with the values warn (default) and enforce.
- **FR-018**: Under enforce, saving or deploying a file with escape-level findings MUST be refused unless an administrator explicitly acknowledges the findings in that request; the acknowledgement and the findings MUST be audited. Root-level findings MUST never block.
- **FR-019**: Depths MUST NOT modify compose files on its own.

**Docs**

- **FR-020**: The user guides MUST explain the three ratings, both policies, how to run stacks without root (`user:` in compose, rootless daemon, user-namespace remapping), and the residual risk that a compromised Depths server could still run privileged commands through its runtime access.

### Key Entities

- **Security rating**: A container's level (ok, root, escape) and its list of findings, derived from its current configuration and the daemon mode. Not stored.
- **Finding**: A machine-readable code, a level, the field it came from and a plain-language sentence.
- **Daemon mode**: rootful, rootless or user-namespace remapped; cached and refreshed with the runtime inventory.
- **Shell policy / compose policy**: start-up settings, documented with the other settings.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of the configurations listed in FR-003 are rated escape in automated tests, and none of them can be shelled into by a non-administrator under either policy.
- **SC-002**: A person can tell which containers in a stack run as root or can reach the host within 5 seconds of opening the stack, without opening any other page.
- **SC-003**: A container reconfigured to escape between page load and shell request is refused in 100% of tested cases.
- **SC-004**: Every shell opened or refused appears in the audit log with its rating, in 100% of tested cases.
- **SC-005**: Lint findings appear in the editor within 1 second of a change to a 500-line compose file.
- **SC-006**: Under the default settings, no existing stack stops working or loses a control other than shells into root and escape containers for non-administrators.

## Assumptions

- Roles are those of spec 050: owner, administrator, user and pending. The owner counts as an administrator.
- The ratings use the configuration the runtime reports for running and stopped containers; images are not scanned.
- The socket proxy in front of the runtime cannot filter what an exec request asks for, so the guard protects against people using Depths, not against a compromised Depths server. The docs say so.
- Network-based restriction of shells and other dangerous actions is specified separately in spec 052 and composes with this guard: both must allow a shell.

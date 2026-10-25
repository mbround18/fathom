# Feature Specification: Depths root boundary and passkeys

**Feature Branch**: `054-depths-root-boundary`

**Created**: 2026-10-24

**Status**: Draft

**Input**: User description: "Harden Depths so a compromised Depths is no longer root on the host (Tier 1 from the hardening review): (1) a policy-enforcing Docker API proxy outside Depths' trust boundary that inspects container-create/update bodies and refuses privileged, host PID/net/IPC/userns, cap_add, devices, and host binds outside the stacks dir and named volumes — the same rules as the 051 compose lint, with an explicit admin-acknowledged exception path; (2) the volume helper runs from its own minimal scratch image containing only the static helper binary, with network none, all caps dropped except the few it needs, read-only rootfs, no-new-privileges, pids and memory limits; (3) passkeys (WebAuthn) as a second factor required for admins, plus step-up re-verification before risky actions (open a shell, deploy a stack the lint flagged or acknowledge an override, write/upload/delete files, change users/roles)."

## Clarifications

### Session 2026-10-25

- Q: How is a runtime-guard exception granted? → A: Only by the operator editing the guard's own settings on the host and restarting it; Depths shows exceptions read-only and cannot grant them.
- Q: Are non-administrators held to step-up? → A: Yes. Every role must register a passkey before its first risky action and confirm it within the freshness window.

## Context

Today anyone who controls the Depths server controls the container runtime, and the runtime is root on
the host. The socket proxy of spec 050 narrows which kinds of request reach the runtime, not what they
ask for, and the escape rules of spec 051 are checked inside Depths, so a compromised Depths skips them.
Sign-in is a single factor at an outside provider, so whoever takes over the owner's provider account
gets the same reach. This spec moves the escape rules to a guard that Depths cannot change, shrinks the
one container Depths runs as root (the volume helper of spec 053), and adds a second factor that a
stolen provider account or session cookie does not carry.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The runtime refuses escapes even when Depths asks (Priority: P1)

An attacker who has taken over the Depths server, or a person deploying a hostile compose file, asks
the runtime for a privileged container, the host's process or network namespace, a dangerous added
capability, a host device, or a mount of a sensitive host path. The request never reaches the runtime:
a guard standing between Depths and the runtime refuses it, says which rule it broke, and records the
refusal in a log that Depths cannot write to or erase. Ordinary stacks, the ones that run as today
with named volumes and folders under the stacks directory, deploy exactly as before.

**Why this priority**: This is the one change that turns "a compromised Depths is root" into "a
compromised Depths can run containers, but not escape them". Everything else in this spec narrows
smaller paths.

**Independent Test**: With Depths replaced by a plain client that talks to the guard directly, send each
escape-level request from the list in FR-003 and confirm every one is refused and logged by the guard,
and that deploying the example stacks and a stack using named volumes and stacks-directory folders works.

**Acceptance Scenarios**:

1. **Given** the guard in front of the runtime, **When** any client sends a container create asking to be privileged, **Then** the runtime never receives it, the client gets a refusal naming the rule, and the guard's log records it.
2. **Given** a compose file that binds `/etc` into a service, **When** it is deployed from Depths, **Then** the deploy fails, Depths shows the guard's reason in plain language next to the service, and Depths' own audit log records the refusal too.
3. **Given** a stack using only named volumes and folders under the stacks directory, **When** it is deployed, **Then** it starts as it did before this spec.
4. **Given** a request of a kind the guard does not recognise, **When** it arrives, **Then** it is refused (deny by default), not passed through.
5. **Given** the guard is not running, **When** Depths tries to reach the runtime, **Then** every request fails and Depths says the runtime guard is unreachable; nothing reaches the runtime around it.

---

### User Story 2 - A deliberate exception for a stack that needs one (Priority: P1)

Some stacks genuinely need a host device (a USB dongle, a GPU) or a host folder outside the stacks
directory (a games library on another disk). The operator grants exactly that, for that stack and
service, in a way a compromised Depths cannot grant itself. Depths shows which exceptions a stack has.

**Why this priority**: Without an exception path, people turn the guard off. The exception must be
narrow and must not be grantable from inside the boundary it protects.

**Independent Test**: Grant one stack's service one host device as an exception, deploy it and confirm it
starts with that device; confirm another stack asking for the same device is refused, and the same
stack asking for a second device is refused.

**Acceptance Scenarios**:

1. **Given** an exception for stack `media`, service `app`, device `/dev/dri`, **When** `media` is deployed, **Then** `app` starts with that device and the guard logs that the exception was used.
2. **Given** that exception, **When** stack `other` asks for `/dev/dri`, **Then** it is refused.
3. **Given** a request to grant an exception, **When** it comes only from the Depths server with no proof from outside it, **Then** no exception is created: exceptions exist only in the guard's own settings on the host, which the operator edits and then restarts the guard.
4. **Given** a stack with exceptions, **When** its page is shown, **Then** each exception is listed with what it allows and since when.
5. **Given** the privileged flag or a mount of the runtime socket, **When** an exception is requested for it, **Then** it cannot be granted; these are never allowed.

---

### User Story 3 - The volume helper can only touch the files it was given (Priority: P2)

When someone browses or edits files in a volume, Depths starts a short-lived helper container that
runs as root so it can read and write files of any owner. That helper now comes from its own image that
contains nothing but the helper program, has no network, keeps only the privileges needed to read,
write and set ownership of files, cannot gain more, cannot write outside the mounts it was given, and
is bounded in processes and memory.

**Why this priority**: The helper is the one container Depths deliberately runs as root with users' data
mounted. Today it starts from the full Depths image, which carries a shell, a download tool and the
runtime client.

**Independent Test**: Start a helper as Depths does and confirm from outside that it has no network, no
shell or other programs in its filesystem, only the listed privileges, a read-only root filesystem,
no way to gain privileges, and the process and memory limits; run the spec 053 file tests against it.

**Acceptance Scenarios**:

1. **Given** a helper started for a volume, **When** its configuration is inspected, **Then** it has no network, only the file-ownership privileges, a read-only root filesystem, the no-new-privileges flag and the process and memory limits.
2. **Given** the helper image, **When** its contents are listed, **Then** it holds only the helper program (and the minimum the program needs to run).
3. **Given** the guard of User Story 1, **When** Depths starts a helper, **Then** the guard allows it without an exception, because the helper asks for nothing escape-level.
4. **Given** every spec 053 operation (list, read, save, upload, download, create, rename, delete, history), **When** it is run with the new helper, **Then** it behaves as before.
5. **Given** a helper asked to start from any image other than the helper image Depths was built with, **When** the request is made, **Then** Depths refuses to start it.

---

### User Story 4 - Administrators prove it is them with a passkey (Priority: P2)

After signing in with their provider, an administrator must also confirm with a passkey (a security key,
or the phone or computer's built-in one). The first time, they are asked to register one before they can
do anything an administrator can do. A stolen provider account or session cookie is then not enough to
act as an administrator.

**Why this priority**: The owner's provider account is the single key to the host today. A second factor
the attacker does not hold closes the most likely takeover.

**Independent Test**: Sign in as an administrator without a passkey and confirm only registration is
offered; register one, sign out and in, and confirm administrator actions need the passkey confirmation;
confirm a copied session cookie used in another browser without the passkey cannot act as administrator.

**Acceptance Scenarios**:

1. **Given** an administrator with no passkey, **When** they sign in, **Then** they are asked to register one and cannot use administrator actions until they do.
2. **Given** an administrator with a passkey, **When** they sign in with the provider, **Then** they must confirm with the passkey before the session counts as an administrator session.
3. **Given** an administrator who cancels or fails the passkey confirmation, **When** they continue, **Then** they have the restricted view of spec 052 and no administrator actions.
4. **Given** an administrator, **When** they open their account page, **Then** they can see their passkeys by name and date, add another, and remove one as long as one remains.
5. **Given** the owner has lost every passkey, **When** they use the recovery path, **Then** they can register a new one, and the recovery is audited. Recovery requires access to the host, not only to Depths.
6. **Given** a user is promoted to administrator, **When** they next act, **Then** they must register a passkey first.

---

### User Story 5 - Risky actions ask again (Priority: P3)

Even within a verified session, the actions that can do the most harm ask for a fresh passkey
confirmation: opening a shell, deploying or saving a stack whose lint found escape-level settings,
acknowledging an override, writing, uploading or deleting files in a volume, and changing users or
roles. One confirmation covers further risky actions for a short while, so a maintenance session is not
a stream of prompts.

**Why this priority**: A session left open, or a session token taken from a verified browser, should
not be enough for the actions that reach the host.

**Independent Test**: In a verified session older than the freshness window, try each risky action and
confirm each asks for the passkey and proceeds only after it; confirm a second risky action inside the
window does not ask again; confirm a cancelled confirmation leaves nothing changed.

**Acceptance Scenarios**:

1. **Given** a verified session whose last confirmation is older than the window, **When** the person opens a shell, **Then** the passkey is asked for first and the shell opens only after it succeeds.
2. **Given** a confirmation within the window, **When** they save a file, **Then** it is not asked again.
3. **Given** a risky action, **When** the confirmation is cancelled or fails, **Then** nothing is changed, and the attempt is audited.
4. **Given** the preview/confirm flow of spec 050, **When** a risky action is confirmed, **Then** the passkey confirmation is part of the confirm step, not a separate screen before the preview.
5. **Given** a person without a passkey who is not an administrator, **When** they try a risky action, **Then** they are asked to register a passkey first, and the action proceeds only after registration and confirmation; step-up applies to every role.

---

### Edge Cases

- A request that sets an escape-level option through a second field that means the same thing (for example a mount given both as a short bind and as a long mount, or a volume whose driver options bind a host path) is judged on every form; any one escape-level form refuses it.
- A container update, an exec, an image build or a plugin install that would grant escape-level reach is judged like a create, and refused by default when the guard cannot judge it.
- A container created before the guard existed keeps running; the guard judges only new requests. Depths' spec 051 ratings still show such containers as escape.
- A host folder outside the stacks directory that an existing stack binds read-write is refused after upgrade unless the operator grants it; the upgrade notes list how to find such stacks before upgrading.
- Paths are judged after resolving `..`, repeated slashes and symbolic links in the configured roots, so `/opt/stacks/../etc` is `/etc`.
- The guard restarting drops in-flight requests; Depths reports them as failed, not as done.
- A passkey confirmation from another site or replayed from an earlier confirmation is rejected.
- Two browsers of the same administrator: each must confirm with a passkey on its own.
- An administrator on an untrusted network (spec 052) still gets only the restricted view after a passkey confirmation; both checks must allow an action.
- The helper image is missing or its identity does not match the server's build: file browsing is unavailable with that reason; nothing falls back to the main image.

## Requirements *(mandatory)*

### Functional Requirements

**Runtime guard**

- **FR-001**: Every request Depths makes to the container runtime MUST pass through a runtime guard that runs outside the Depths server's control: Depths MUST NOT be able to change the guard's rules, exceptions or log, or reach the runtime by any other route.
- **FR-002**: The guard MUST refuse any request kind it does not explicitly allow (deny by default), and MUST fail closed when it cannot read or judge a request.
- **FR-003**: The guard MUST refuse container creates and updates that are privileged; that use the host's process, network, IPC, user or cgroup namespace; that add a capability outside the runtime's default set; that pass host devices; that disable the security profile or the no-new-privileges protection; that mount a container-runtime socket; or that mount a host path outside the allowed roots. The escape-level settings of spec 051 FR-003 MUST all be covered.
- **FR-004**: The allowed host roots MUST default to the stacks directory and the runtime's named volumes; the operator MAY add more roots in the guard's settings. The sensitive paths of spec 051 FR-003 MUST be refused even if an added root contains them.
- **FR-005**: The guard MUST judge every form a request can use to ask for the same reach (short and long mount forms, volume driver options that bind a host path, container updates, execs that ask for privilege or another user, image builds that ask for elevated build rights), and refuse if any form is escape-level.
- **FR-006**: Every refusal MUST return a reason that names the rule and the offending setting, and MUST be recorded in the guard's own log with the time, the request kind and the reason. Allowed requests that used an exception MUST also be recorded.
- **FR-007**: Depths MUST translate guard refusals into its usual refusal display, next to the stack and service concerned, and MUST audit them in its own log as well.
- **FR-008**: Exceptions MUST name one stack, one service and one setting each (a device path, a host root, a capability), MUST NOT be grantable for the privileged flag or a runtime socket, and MUST be grantable only by a path that a compromised Depths server cannot use on its own: the operator edits the guard's settings on the host and restarts the guard. Depths MUST have no way to create, change or remove an exception.
- **FR-009**: Depths MUST show a stack's exceptions on its page, read-only.
- **FR-010**: The example compose file and install guide MUST place the guard between Depths and the runtime by default, with no route from Depths to the runtime that bypasses it.

**Volume helper**

- **FR-011**: The volume helper MUST run from its own image that contains only the helper program and what it strictly needs to run, built and released together with the server, and identified to the server by content identity, not by a movable name.
- **FR-012**: Every helper container MUST run with no network, all privileges dropped except those needed to read, write and set ownership of files of any owner, a read-only root filesystem, the no-new-privileges protection, and limits on processes and memory.
- **FR-013**: The server MUST refuse to start a helper from any image other than the one it was built with, and MUST NOT fall back to the main image.
- **FR-014**: All spec 053 file operations MUST keep their behaviour and tests with the new helper.

**Passkeys and step-up**

- **FR-015**: Depths MUST support registering passkeys (security keys and platform authenticators) per person, listing them by name and date, adding more, and removing one as long as one remains.
- **FR-016**: A session MUST NOT count as an administrator session until the person has confirmed with a passkey in that session; an administrator with no passkey MUST be required to register one before any administrator action.
- **FR-017**: The actions in User Story 5 (open a shell; save or deploy a stack with escape-level lint findings; acknowledge an override; write, upload, rename or delete files in a volume; restore a file version; change users, roles or network whitelists) MUST require a passkey confirmation no older than the freshness window, 5 minutes by default and configurable by the operator, for every role; a person without a passkey MUST register one before their first risky action.
- **FR-018**: Passkey confirmations MUST be bound to Depths' public address, to the session and to a single-use challenge, and MUST be rejected when replayed or used from another site.
- **FR-019**: The passkey confirmation for a risky action MUST be part of the existing confirm step, and a failed or cancelled confirmation MUST change nothing.
- **FR-020**: Every passkey registration, removal, confirmation, failure and recovery MUST be audited.
- **FR-021**: The owner MUST have a recovery path for lost passkeys that requires access to the host (for example a one-time code produced on the host), not only to Depths or the provider account.
- **FR-022**: The passkey requirement and the network trust of spec 052 MUST both allow an action; neither replaces the other.

**Docs**

- **FR-023**: The guides MUST explain the guard and what it refuses, how to grant and review exceptions, how to find stacks that need one before upgrading, passkey registration and recovery, and the residual risk that remains (a compromised Depths can still run, stop and delete non-escape containers and read stack files).

### Key Entities

- **Guard rule**: A named check on a runtime request (for example "privileged", "host namespace", "host path outside allowed roots") with the setting it inspects. Fixed in the guard; not editable from Depths.
- **Exception**: Stack, service, the single setting it allows, who granted it and when. Held by the guard.
- **Guard log entry**: Time, request kind, stack and service when known, outcome, rule or exception. Held by the guard.
- **Passkey**: Owner (a person), display name, created and last-used times, and the public credential. The private key never leaves the authenticator.
- **Verification**: The time a session last confirmed with a passkey; decides administrator status and the freshness window.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of the escape-level configurations in FR-003 and FR-005, sent straight to the guard by a client other than Depths, are refused and logged in automated tests.
- **SC-002**: Every example stack and every stack in the test fixtures that uses only named volumes and stacks-directory folders deploys unchanged with the guard in place.
- **SC-003**: With the guard stopped, 0 requests reach the runtime.
- **SC-004**: The helper image holds no program other than the helper, and every spec 053 file test passes against it.
- **SC-005**: A copied session cookie without the passkey cannot perform any administrator or risky action in 100% of tested cases.
- **SC-006**: An administrator completes a risky action including the passkey confirmation in under 10 seconds of interaction, and a second risky action within the window needs no extra step.
- **SC-007**: An owner who lost every passkey regains administrator access through host recovery in under 5 minutes, and the recovery is in the audit log.

## Assumptions

- Roles, preview/confirm and the audit log are those of spec 050; ratings, lint and policies are those of spec 051; network trust is that of spec 052; the volume helper and its operations are those of spec 053.
- The guard judges requests, not images: it does not scan image contents or stop a non-escape container from doing harm inside itself.
- Containers that already exist when the guard is introduced are not stopped or changed.
- Stacks that build images get only ordinary build rights; elevated build rights are refused like any other escape-level request.
- Passkeys come from the browser's built-in support; no hardware is shipped or required beyond what the person already has (a phone or computer with a built-in authenticator counts).
- The provider sign-in of spec 050 stays the first factor; passwords are not added.
- Moving the stacks themselves to a rootless runtime is out of scope; spec 051's guide already covers it.
- The owner's deployment on steam is upgraded only after the operator has reviewed which existing stacks need exceptions.

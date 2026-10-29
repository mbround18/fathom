# Feature Specification: Depths image updates across many stacks (Watchtower-compatible)

**Feature Branch**: `062-depths-auto-update`

**Created**: 2026-10-28

**Status**: Draft

**Input**: User description: "for Depth i do want watch tower support which is a separate spec esp for many stack management". Clarified: Depths does Watchtower-style updates itself, honouring Watchtower's labels, rather than driving a Watchtower container.

## Clarifications

### Session 2026-10-28

- Q: Integrate a real Watchtower container, or do Watchtower-style updates natively? → A: Natively, and Watchtower-compatible. Depths runs the checks and updates itself, audited like any other operation, and reads Watchtower's labels so stacks already labelled for Watchtower behave the same. No second container needs the Docker socket, and the original Watchtower project is archived, so nothing depends on it.
- Q: Can checks cost zero pulls? → A: Checks of unchanged tags count zero pulls. Each newly published image costs at most one counted pull, once; on the containerd image store the running image costs one more, once (SC-007, FR-003).
- Q: How does Depths reach private registries? → A: Through a plain `config.json` mounted into the Depths container, with `DOCKER_CONFIG` pointing at it. Credential helpers are not supported (Assumptions).
- Q: Who authorises an unattended update, when no one is signed in? → A: The administrator who set the stack to Auto, through that passkey-confirmed change. Their policies pause if they stop being an administrator (FR-025).
- Q: Are the `depths-keep/…` image tags acceptable? → A: Yes. They are visible in `docker images` and documented (FR-028).

## Why

Someone running thirty stacks can't press Pull on each one every week, so they run Watchtower. Watchtower then updates containers out of sight. It works container by container rather than stack by stack, so it ignores the compose file Depths treats as the truth, and it holds the Docker socket in a second container. When an update breaks a service, nothing records what changed or offers a way back.

Depths already pulls and recreates stacks with a preview and an audit entry (spec 050). This feature does the same across many stacks on a schedule:
- it finds which services have newer images;
- it shows them in one place;
- it lets an administrator update selected stacks in one reviewed step;
- for stacks the administrator opts in, it updates them unattended, one at a time, inside a maintenance window, and rolls back when a service doesn't come back healthy.

It honours the labels people already put on containers for Watchtower, so moving from Watchtower to Depths changes nothing in their compose files.

Unlike Keel (spec 061), which only ever updates itself after an owner confirms, Depths manages other people's stacks, and unattended updates are what an operator of many stacks wants. The safeguards are a window, rollback, the audit log and a hold on anything that would weaken containment.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See every stack that has a newer image (Priority: P1)

An administrator opens **Updates**. It lists every service in every stack with its image, the digest it runs, and its update state: up to date, update available, held (with reason), excluded (with reason), updating, failed, or not checked. It is filterable by state and stack. A badge on the navigation entry shows how many stacks have updates. Users see the same states, only for their assigned stacks, on each stack's page.

**Why this priority**: Knowing what is stale is most of the value, and nothing changes on the host to get it. It also replaces Watchtower's monitor-only mode.

**Independent Test**: With update checks on and three stacks, push a newer image for one service's tag to a test registry, run a check, and see that service as "update available", with its stack counted in the badge, and the other services as up to date. Sign in as a user assigned only another stack and see none of it.

**Acceptance Scenarios**:

1. **Given** update checks are off (the default), **When** Depths runs, **Then** it makes no registry requests. Updates says checks are off and how an administrator turns them on.
2. **Given** checks are on, **When** a check runs (on its schedule, or **Check now**), **Then** each service's state reflects whether the tag it uses now points to a different image than the one its container runs, for the host's architecture.
3. **Given** a service whose image is pinned by digest, or that is built from a build context, **When** it is listed, **Then** it shows as excluded with that reason, and no check is made for it.
4. **Given** a service labelled `com.centurylinklabs.watchtower.enable=false`, **When** it is listed, **Then** it shows as excluded "by label", and Depths never updates it.
5. **Given** a registry refuses, rate-limits or can't be reached, **When** the check finishes, **Then** the affected services show "check failed" with the reason and keep their last known state. A failure never reports an update.
6. **Given** a user assigned some stacks, **When** they open a stack, **Then** they see its services' update states. The Updates page, other stacks and update policies are administrator-only.

---

### User Story 2 - Update many stacks in one reviewed step (Priority: P1)

From Updates, an administrator selects several stacks with updates and chooses **Update selected**. The preview lists, per stack, which services will be pulled and recreated, the old and new image details, and the order. After they confirm, Depths updates the stacks one at a time. Each stack is the same pull-and-recreate as spec 050's Pull and Up, streamed live and audited per stack. A stack that fails doesn't stop the others; the summary shows each outcome.

**Why this priority**: This is the manual path that replaces pressing Pull thirty times. It is safe on day one, before anyone opts into unattended updates.

**Independent Test**: With updates available for three stacks, select all three, confirm the preview, and see them updated in order, each with output and an audit entry. Make the second fail and see the first and third still updated, with the summary naming the failure.

**Acceptance Scenarios**:

1. **Given** selected stacks, **When** the preview opens, **Then** it lists each stack, the services that will be recreated, old and new digest, version labels when present, and states that only services with a newer image are recreated.
2. **Given** a stack with an operation in progress, **When** the batch reaches it, **Then** it is skipped with that reason rather than queued behind or run alongside (spec 050 US2).
3. **Given** a stack whose lint, after pulling, finds that a new image would run a previously non-root service as root, or that the runtime guard (spec 054) would refuse it, **When** the batch reaches it, **Then** it is not recreated. It is held with the reason, and the pulled image is kept for review.
4. **Given** the session is on an untrusted network (spec 052), **When** the administrator tries to update, **Then** it is refused like any pull. A passkey confirmation (spec 054) is asked for as for other risky actions.
5. **Given** a user on one of their assigned stacks, **When** an update is available, **Then** they can update that stack with spec 050's Update, which follows this feature's update steps while checks are on, so the same hold rules apply (FR-026).

---

### User Story 3 - Opt stacks into unattended updates (Priority: P2)

An administrator gives each stack an update policy:
- **Off**: never checked.
- **Notify**: checked and listed. This is the default once checks are on.
- **Auto**: checked and updated unattended, inside the maintenance window.

They set a host-wide window (for example daily 03:00–05:00 host time) and may override it per stack. In the window, Depths updates opted-in stacks one at a time. It checks each recreated service comes back healthy, or running if it has no health check, within a timeout. If not, it returns the service to the previous image and marks the stack failed. Every unattended update is audited under "Depths (scheduled)", naming the administrator who set the policy. That administrator's passkey-confirmed policy change is what authorises the unattended runs.

**Why this priority**: This is Watchtower's main job. It comes after US1 and US2 because it changes hosts without anyone watching, so it must rest on the same checks and holds.

**Independent Test**: Set one stack to Auto with a window covering now. Publish a newer image, and see Depths update it inside the window with an audit entry. Publish an image whose service exits at once, and see Depths roll back to the previous image, mark the stack failed, and not retry that digest. Set another stack to Notify and see it listed but not touched.

**Acceptance Scenarios**:

1. **Given** a stack on Auto and an update available, **When** the window opens, **Then** Depths updates it unattended, one stack at a time, and not outside the window.
2. **Given** a recreated service that doesn't become healthy, or running when it has no health check, within the timeout (default 2 minutes), **When** the timeout passes, **Then** Depths recreates it on the previous image, the stack shows "rolled back", and that digest is not tried again automatically until a newer one appears or an administrator retries.
3. **Given** a stack on Auto whose update would be held (US2 scenario 3), **When** the window runs, **Then** it is held, never applied, and shows on Updates as needing review.
4. **Given** the window closes mid-batch, **When** the stack being updated finishes, **Then** Depths stops and leaves the rest for the next window.
5. **Given** an administrator changes a policy or the window, **When** they save, **Then** it needs a passkey (spec 054) and is audited with before and after.
6. **Given** Depths restarts during an unattended update, **When** it comes back, **Then** it reports the stack's real state and records the update as interrupted rather than succeeded.
7. **Given** a service rolled back to its previous image that doesn't come back healthy either (for example because the new image migrated its data), **When** the second timeout passes, **Then** the stack shows "rollback failed" with the output, and Depths makes no further attempt; it waits for a person.
8. **Given** a stack on Auto whose policy was set by an administrator who has since been demoted or removed, **When** the window opens, **Then** the stack is not updated. Updates shows the policy as paused, "author no longer an administrator", until another administrator confirms it.

---

### User Story 4 - Stacks labelled for Watchtower keep working (Priority: P2)

An operator moving from Watchtower doesn't touch their compose files. Depths reads Watchtower's labels and maps them to its policies. Labels Depths doesn't act on are shown, with the reason. A Watchtower container still running on the host is detected, and Depths won't run unattended updates alongside it.

**Why this priority**: It makes the switch safe and costless, which is what "Watchtower support" means for someone with many stacks.

**Independent Test**: Import stacks labelled `enable=false`, `monitor-only=true` and `scope=media`, and see their effective policies as excluded, Notify and (with Depths' scope set to `media`) included. Start a Watchtower container, and see Updates warn and Auto pause until an administrator resolves it.

**Acceptance Scenarios**:

1. **Given** `com.centurylinklabs.watchtower.enable=false`, **When** Depths computes the effective policy, **Then** the service is excluded whatever the stack policy says.
2. **Given** the host-wide setting "only labelled services" is on, **When** a service lacks `com.centurylinklabs.watchtower.enable=true`, **Then** it is excluded, matching Watchtower's label-enable mode.
3. **Given** `com.centurylinklabs.watchtower.monitor-only=true`, **When** the policy is Auto, **Then** the service is treated as Notify.
4. **Given** `com.centurylinklabs.watchtower.scope=<name>`, **When** Depths has a scope configured, **Then** only services with a matching scope are included. With no scope configured, scoped services are included and their scope is shown.
5. **Given** lifecycle-hook labels (`lifecycle.pre-update` and similar) or `depends-on`, **When** a service is listed, **Then** Depths shows them as not run, because a hook is a command run inside the container, which Depths only allows through its guarded shell (spec 051). Stack order comes from the compose file instead.
6. **Given** a running Watchtower container on the host, **When** an administrator opens Updates, **Then** a banner names it and says both would update the same containers. Auto updates stay paused until the administrator confirms that Watchtower is stopped or set to monitor-only. Depths never stops it itself.
7. **Given** a stack policy set in Depths, **When** it differs from the labels, **Then** the label wins where it is more restrictive (exclude, monitor-only), and the UI says which one applied. Depths never edits a compose file to change labels.
8. **Given** `com.centurylinklabs.watchtower.no-pull=true`, **When** Depths computes the effective policy, **Then** the service is excluded "no-pull by label", because Depths only updates by pulling.

---

### User Story 5 - Old images are cleaned up, but never the way back (Priority: P3)

After a successful update, Depths can remove the image the service used before. Administrators turn this on host-wide. Depths keeps the image a service rolled back to, and the most recent previous image of each service, until the next successful update. It holds them under its own local tags, `depths-keep/<stack>:<service>-previous` and `depths-keep/<stack>:<service>-held`, so that a `docker image prune` can't remove the way back. Those tags show in `docker images`.

**Why this priority**: Saves disk on hosts with many stacks; Watchtower's `--cleanup` is widely used. It's last because it deletes things.

**Independent Test**: With cleanup on, update a stack twice and see only the image from two updates ago removed, with an audit entry naming it. With cleanup off, see nothing removed.

**Acceptance Scenarios**:

1. **Given** cleanup is on and an update succeeds, **When** the stack is healthy, **Then** images older than the immediately previous one for that service are removed, if nothing else uses them. Each removal is audited.
2. **Given** an image used by any other container, or by a stack outside the stacks directory, **When** cleanup runs, **Then** it is kept.
3. **Given** cleanup is off (the default), **When** updates run, **Then** no image is removed.
4. **Given** an image under a `depths-keep/…` tag that is a stack's current rollback target, **When** cleanup runs, **Then** it is kept.

### Edge Cases

- **Tag points to an older image**: shown as "tag moved back", never applied automatically.
- **Same image, new digest** (a re-push of identical content for another architecture, or an index change): compared by the image the host would actually run, so it's no update.
- **Private registries**: use the credentials in the `config.json` mounted into the Depths container (Assumptions). Depths never shows, logs or audits them. A registry Depths can't authenticate to, or that won't say which image a tag points to without a download, shows "can't check" for its services; Depths never pulls to find out.
- **Rate limits**: checks of unchanged tags use requests that don't count as pulls, spread across the check interval, and back off per registry after a refusal.
- **Several stacks use the same tag**: tags are shared by the whole host. If one stack rolls back, the tag points at the old image again, so another stack already updated to the new image keeps running it but shows "update available" again, and would get the old image on its next recreate. `docker compose pull` has the same exposure today; it is documented, not prevented.
- **Keep tags in `docker images`**: the `depths-keep/…` tags are Depths' way back (US5). Removing them by hand removes the way back for that service.
- **Several services in one stack share an image**: one check, and all of them are recreated together.
- **A service is scaled to several replicas**: all replicas are recreated, and each must be healthy.
- **Depths' own container**: shown with its update state but never updated by Depths itself, because recreating it mid-update would cut the update off. The page shows the manual steps instead.
- **Stack edited between preview and update**: the update is refused as stale and must be previewed again, as for any spec 050 operation.
- **Host clock changes or daylight-saving transitions**: the window is in host local time; a skipped or repeated hour neither runs twice nor skips the day.

## Requirements *(mandatory)*

### Functional Requirements

**Checking**

- **FR-001**: Depths MUST make no registry request unless an administrator turns update checks on. It MUST be off by default.
- **FR-002**: With checks on, Depths MUST check every service whose policy isn't Off on a configurable interval (default 6 hours, minimum 1 hour), and on demand by administrators at most once a minute.
- **FR-003**: A check MUST compare the image the service's tag resolves to for the host's architecture with the image its container runs, and MUST read the new image's version and revision labels when present. Checking a tag that hasn't moved MUST use only requests that don't count as pulls. Each newly published image MAY cost at most one counted pull, once, however many services use it. A registry that can't answer without a counted request for an unchanged tag MUST show "can't check" rather than be pulled.
- **FR-004**: Services pinned by digest, built from a build context, or excluded by label MUST be excluded with their reason and MUST NOT be checked.
- **FR-005**: A failed check MUST keep the service's last known state, show the reason, and MUST NOT report an update.

**Seeing**

- **FR-006**: Administrators MUST have an Updates view of every stack's services, with update state and reason, filterable by state and stack, and a navigation badge counting stacks with updates.
- **FR-007**: Users MUST see update states only for services of their assigned stacks, on the stack's page.

**Updating**

- **FR-008**: Administrators MUST be able to update selected stacks in one previewed, confirmed step. The stacks run one at a time, each audited separately, and a failure MUST NOT stop the rest.
- **FR-009**: Only services with a newer image MAY be recreated. The compose file MUST NOT be changed.
- **FR-010**: Before recreating, Depths MUST hold any update whose new image would make a previously non-root service run as root, or that the runtime guard would refuse. The hold MUST show the reason and MUST NOT be applied automatically.
- **FR-011**: Updates MUST follow spec 052's network trust and spec 054's passkey rules for pulls and risky actions, and spec 050's one-operation-per-stack rule.
- **FR-026**: While checks are on, spec 050's Update on a stack MUST use this feature's update steps: pull the previewed image by digest, run the hold checks, then point the tag at it and recreate. With checks off, it stays spec 050's pull then up.

**Unattended**

- **FR-012**: Each stack MUST have a policy (Off, Notify, Auto), with Notify the default once checks are on. Administrators MUST be able to set a host-wide maintenance window, and per-stack windows that override it.
- **FR-013**: Auto updates MUST run only inside the window, one stack at a time, and MUST stop starting new stacks when the window closes.
- **FR-014**: After recreating, Depths MUST wait for each recreated service to be healthy (or running, without a health check) within a timeout, default 2 minutes, and MUST otherwise return it to its previous image and mark the stack rolled back. If the service isn't healthy on the previous image either, Depths MUST mark the stack "rollback failed", show the output and make no further attempt.
- **FR-015**: A rolled-back digest MUST NOT be retried automatically until a newer digest appears or an administrator retries.
- **FR-016**: Unattended updates MUST be audited as "Depths (scheduled)", with the administrator who set the policy, the stack, the services, old and new image and the outcome.
- **FR-017**: Changing policies, windows, the scope, label-only mode and cleanup MUST be administrator-only, need a passkey, and be audited with before and after.
- **FR-018**: An update interrupted by Depths restarting MUST be recorded as interrupted, with the stack's real state.
- **FR-025**: Unattended updates have no signed-in session, so spec 052's network trust can't be checked when they run. They MUST be authorised by the policy change that set the stack to Auto, which itself needed a trusted network and a passkey (FR-017). An Auto policy whose author is no longer an administrator MUST be paused, and shown as such, until another administrator confirms it.

**Watchtower compatibility**

- **FR-019**: Depths MUST honour these Watchtower labels: `enable` (and a host-wide "only labelled services" mode), `monitor-only`, `scope` and `no-pull` (which excludes the service). Labels MUST win over Depths' policy where they are more restrictive.
- **FR-020**: Depths MUST NOT run Watchtower lifecycle hooks. It MUST show hook and `depends-on` labels as not run, with the reason.
- **FR-021**: Depths MUST detect a running Watchtower container on the host, warn administrators, and pause Auto updates until an administrator confirms. It MUST NOT stop or change that container itself.
- **FR-022**: Depths MUST NOT edit compose files or labels to apply a policy.

**Cleanup**

- **FR-023**: Administrators MUST be able to turn on removal of old images after successful updates. It MUST be off by default. It MUST keep each service's immediately previous image and any image in use, and MUST audit each removal.
- **FR-027**: Cleanup MUST only remove images Depths itself recorded as a service's previous image. It MUST NOT remove an image under a `depths-keep/…` tag while that image is a stack's current rollback target.
- **FR-028**: Depths MUST keep each updated service's previous image, and any held image, under local tags `depths-keep/<stack>:<service>-previous` and `depths-keep/<stack>:<service>-held`, so a prune can't remove them. These tags are visible in `docker images` and MUST be documented.

**Itself**

- **FR-024**: Depths MUST NOT update its own container. It MUST show its own update state with manual steps.

### Key Entities

- **Update policy**: stack, Off / Notify / Auto, optional window, who set it and when. Paused when its author is no longer an administrator.
- **Host update settings**: checks on or off, interval, default window, scope, label-only mode, cleanup on or off, rollback timeout, Watchtower acknowledgement.
- **Service update state**: stack, service, image reference, running image, available image (digest, version, revision), state and reason, last checked, last outcome.
- **Update run**: who or "Depths (scheduled)" with the policy's author, stacks in order, per-stack outcome (updated, held, skipped, failed, rolled back, interrupted), each tied to its audit entry and output.
- **Rejected digest**: service and digest that rolled back, so it isn't retried.
- **Keep tag**: a local `depths-keep/…` tag naming a service's previous or held image, and the image it points to.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With default settings, Depths makes zero registry requests over 24 hours.
- **SC-002**: An administrator with 30 stacks sees which have newer images, on one page, within one check interval of publication, without opening any stack.
- **SC-003**: Updating 10 stacks takes one preview and one confirmation, and produces 10 audit entries with outcomes.
- **SC-004**: A deliberately broken image on an Auto stack is rolled back within the health timeout plus 1 minute, and is not retried in the next window.
- **SC-005**: An update that would turn a non-root service into a root one is applied 0 times without an administrator.
- **SC-006**: Stacks already labelled for Watchtower need no file changes. Their effective policy matches Watchtower's behaviour for `enable`, `monitor-only` and `scope` in 100% of the compatibility test cases.
- **SC-007**: Over a week of 6-hourly checks across 30 stacks, checks of unchanged tags count zero image pulls against registry limits, and each newly published image costs at most one counted pull, once. On Docker's containerd image store, the image a service was already running costs one more counted pull, once, the first time its tag moves (contracts/registry.md "Request budget").

## Assumptions

- "Watchtower support" means Watchtower-compatible behaviour inside Depths, not running or driving a Watchtower container (Clarifications).
- Updates follow the tag a service already uses. Moving to a newer tag (for example `1.4` to `1.5`) is out of scope; that's a dependency-update tool's job.
- External notifications (email, chat webhooks), which Watchtower sends, are out of scope for this spec. Updates are visible in Depths and the audit log. A notification channel can follow as its own feature.
- Health means the container's own health check when it has one, otherwise "running" for the timeout. Depths doesn't probe services itself.
- Rollback returns a service to the image it ran before. It can't undo a database migration the new image already ran. If the old image then fails too, the stack is left "rollback failed" for a person (FR-014). Operators who need that keep such stacks on Notify.
- Private registries need a plain `config.json` mounted into the Depths container, with `DOCKER_CONFIG` set to its directory. The Docker daemon holds no registry credentials, and Depths' container can't see the host user's `~/.docker`. Credential helpers (`credsStore`, `credHelpers`) are not supported, because the helper programs aren't in the container: services on those registries show "can't check". Spec 050's pulls of private images have the same gap today, and the same mount fixes both.
- Policies live in Depths' own state beside the users list, never in compose files, so Dockge compatibility is untouched (spec 050).
- One host per Depths, as in spec 050.
- Keel does not get unattended updates (spec 061); this is a deliberate difference, because Keel updates its own host and Depths updates the stacks it manages.

## Dependencies

- Spec 050: stacks, Pull and Up, the one-operation-per-stack rule, roles and the audit log.
- Spec 051: root and escape-capability ratings used to hold updates; the guarded shell, which is why hooks aren't run.
- Spec 052: network trust for pulls.
- Spec 054: the runtime guard and passkey step-up. US1 doesn't need 054 and can ship first. US2's batch confirm and US3's policy changes need 054's passkey step-up.
- Spec 061: shares the registry client with this feature. Whichever of the two is built first creates it.

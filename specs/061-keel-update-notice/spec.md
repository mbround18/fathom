# Feature Specification: Keel update notice and self-update

**Feature Branch**: `061-keel-update-notice`

**Created**: 2026-10-28

**Status**: Draft

**Input**: User description: "Keel: update notice and self-update for a Docker-deployed Keel server. Keel knows what it runs (version, revision, image digest); checks the registry for a newer image of its tag behind a runtime flag that is off by default; tells instance owners in System Settings; and lets an owner update through the System workspace with preview, passkey and audit. No silent updates. Without a System workspace, the notice shows manual instructions."

## Why

Keel runs as a container image that is rebuilt often. Today nobody can tell from Keel which build is running, and nobody learns that a newer one exists unless they check the registry by hand. When they do update, it means a shell on the host, `docker compose pull`, and hoping `latest` meant what they thought it did.

Keel already has the pieces to do better. Spec 060 gave owners a System workspace that can run the `keel-server` play against Keel's own host, and made results survive Keel restarting. This feature uses them: Keel says what it runs, notices when something newer is published, tells the owner, and lets the owner update in one reviewed, audited step. It never updates by itself.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Keel says exactly what it is running (Priority: P1)

An owner or admin opens System Settings → System and sees the running Keel's version, source revision, image digest and build date. The deployment pins Keel to an image digest rather than a moving tag, so the running build is unambiguous and the previous digest is all a rollback needs.

**Why this priority**: Every other story compares against this. It is also useful on its own: support questions start with "which build?".

**Independent Test**: Build an image with a known version and revision, deploy it with its digest supplied, open the System tab, and see all four values. Deploy a build without a digest and see the digest shown as unknown, with the version and revision still shown.

**Acceptance Scenarios**:

1. **Given** an image built with a version and revision and deployed with its digest, **When** an owner or admin opens the System tab, **Then** they see the version, revision (short form, full on hover or copy), digest (short form, full on copy) and build date.
2. **Given** a build without a version or revision (a local development build), **When** the tab opens, **Then** those values read "development build" instead of placeholders such as `0.0.0` or `unknown`.
3. **Given** the deployment did not supply a digest, **When** the tab opens, **Then** the digest reads "unknown" with a one-line hint that the deployment should pin a digest.
4. **Given** the owner's Ansible role deploys Keel, **When** it runs, **Then** it resolves the configured tag to a digest, deploys Keel pinned to that digest, and passes the digest to Keel. The previous digest is recorded so a rollback can name it.
5. **Given** any signed-in user, **When** they open About (or the equivalent), **Then** they see the version only, not the digest or revision.

---

### User Story 2 - Update checks are off unless an owner turns them on (Priority: P1)

Keel makes no outbound call to any registry or release site unless the deployment turns the update check on. Air-gapped and privacy-sensitive installs get nothing new by default.

**Why this priority**: A server that calls out without being asked breaks trust and air-gapped installs. This must hold before the check exists.

**Independent Test**: Start Keel with default settings on a network that logs outbound connections. Leave it running across several check intervals and press nothing; no registry or release-site connection is attempted. The System tab says update checks are off and how to turn them on.

**Acceptance Scenarios**:

1. **Given** the default configuration, **When** Keel runs for any length of time, **Then** it makes no request to a registry or release site, and the System tab says checks are off.
2. **Given** checks are turned on in the deployment configuration, **When** Keel starts, **Then** it checks once shortly after start and then at the configured interval (default every 6 hours).
3. **Given** an invalid check setting (an unknown mode, an interval under 1 hour, a malformed image reference), **When** Keel starts, **Then** it refuses to start and names the setting.

---

### User Story 3 - Owners learn that a newer image is published (Priority: P1)

With checks on, Keel compares the digest its configured tag points to on the registry with the digest it is running. When they differ, owners see a dot on the System Settings entry in the navigation and an update card on the System tab: what is running, what is available (version, revision, build date, digest), when Keel last checked, and a link to the release notes when the new image carries a release version. Owners can also press **Check now**.

**Why this priority**: This is the feature the user asked for. It is valuable without the one-click update: the owner can update by hand once they know.

**Independent Test**: Run Keel against a test registry. Push a newer image to the watched tag. Press Check now as an owner and see the dot and card with the new image's details. Sign in as an admin and see at most a read-only line, with no button. Sign in as a user and see nothing.

**Acceptance Scenarios**:

1. **Given** checks are on and the watched tag points to the running digest, **When** a check runs, **Then** the System tab says Keel is up to date, with the time of the check.
2. **Given** the watched tag points to a different image, **When** a check runs, **Then** connected owners are told at once, without reloading: a dot appears on the System Settings entry, and the System tab shows the update card.
3. **Given** the new image carries a release version, **When** the card shows, **Then** it links to that version's release notes. **Given** it carries only a revision, **Then** the card shows the revision and says there are no release notes for untagged builds.
4. **Given** an owner presses Check now, **When** the check finishes, **Then** the card updates. Check now is limited to once a minute per instance; extra presses show when the next check is allowed.
5. **Given** an admin who isn't an owner, **When** an update is available, **Then** they see a read-only line "A newer Keel is available; an owner can update it", with no dot, no digest and no actions. **Given** a user, **Then** they see nothing.
6. **Given** an owner dismisses the dot, **When** they reload, **Then** the dot stays dismissed for that digest and returns only when a different newer digest appears. The card on the System tab stays.
7. **Given** a check fails (registry unreachable, rate-limited, the tag doesn't exist, credentials refused), **When** the owner opens the System tab, **Then** they see the last successful result, if any, the time and reason of the failure, and the next attempt time. A failure never shows an update as available.

---

### User Story 4 - An owner updates Keel in one reviewed step (Priority: P2)

On the update card, an owner presses **Update Keel**. Keel shows a preview: the run that will happen (the `keel-server` profile on the Keel host in the System workspace), the digest it will deploy, the digest it replaces, and that Keel will restart after the run reports. The owner confirms with a passkey. The run appears in Runs like any other. Keel restarts on the new image after the run reports, and when it is back the System tab shows the new digest and the run as finished.

**Why this priority**: It removes the last routine reason to open a shell on the Keel host, but it depends on spec 060's System workspace, and the notice is useful without it.

**Independent Test**: On a test instance with a configured System workspace and an enrolled Keel host, publish a newer image, update from the card, and see: preview, passkey, a run in Runs, Keel restarting, and afterwards the new digest on the System tab, the run finished, and audit entries for the request and the result.

**Acceptance Scenarios**:

1. **Given** an available update and a configured System workspace with the Keel host enrolled and `keel-server` allowed, **When** an owner presses Update Keel, **Then** they see a preview naming the host, the profile, the current and new digest, and the restart, and confirming needs a passkey.
2. **Given** the owner confirms, **When** the run is queued, **Then** it pins exactly the previewed digest, never the tag, so an image pushed after the preview is not what gets deployed. The request is audited with both digests.
3. **Given** the run reaches Keel's restart, **When** it happens, **Then** the run reports first and the restart comes after (spec 058). If Keel stops before the result lands, the runner delivers it once Keel is back (spec 060).
4. **Given** Keel is back on the new digest, **When** an owner opens the System tab, **Then** it shows the new digest as running, the card is gone or says up to date, and the run is finished. A `keel_updated` audit entry records old and new digest.
5. **Given** an update run is already queued or running, **When** an owner presses Update Keel, **Then** they see the existing run instead of starting a second one.
6. **Given** the run fails before the restart, **When** it finishes, **Then** Keel keeps running the old digest, the card says the update failed with a link to the run, and the owner can try again.
7. **Given** Keel does not come back within the expected time, **When** it does come back (on either digest), **Then** the System tab shows what it is running and the run's outcome as the runner recorded it, and points to `make keel-server` on the host as the repair path.
8. **Given** an admin, a user or an API token, **When** they request an update directly, **Then** it is refused and audited.

---

### User Story 5 - Without a System workspace, the owner gets instructions (Priority: P3)

When no System workspace is configured, the update card still shows, with what to do by hand: the digest to pin, and the commands to pull and restart. It also says that configuring the System tab enables one-click updates.

**Why this priority**: It covers installs that don't manage the Keel host from Keel, which is every install until spec 060's System tab is set up.

**Independent Test**: With checks on and no System workspace, publish a newer image and see the card with copyable instructions and no Update button.

**Acceptance Scenarios**:

1. **Given** an available update and no System workspace, **When** an owner opens the System tab, **Then** the card shows the new digest and copyable steps for the deployment repository and for a plain compose install, and a link to set up the System tab.
2. **Given** the System workspace exists but the Keel host isn't enrolled or `keel-server` isn't allowed, **When** the owner opens the card, **Then** it says which step is missing instead of showing a disabled button without reason.

---

### Edge Cases

- **Tag moved backwards**: the watched tag now points to an image older than the running one (by build date or version). The card says the tag points to an older build and does not offer it as an update. Owners can still see it.
- **Running digest unknown**: Keel can't compare digests. It compares revisions instead, and says so; if those are also unknown, it reports "can't tell" rather than "update available".
- **Multi-architecture images**: the comparison uses the digest for the architecture Keel runs on, or the index digest when the deployment pinned the index. Keel never reports an update only because another architecture's image changed.
- **Same revision, new digest** (a rebuild of the same source): shown as "rebuilt", with the build dates, not as a new version.
- **Registry rate limits**: checks use requests that do not count as image pulls, back off after a refusal, and never retry more than once per interval.
- **Private registries**: credentials, when configured, come from deployment configuration and are never shown, logged or audited.
- **Several Keel servers** (future): each checks and reports for itself. Out of scope beyond not breaking.
- **Clock skew**: "last checked" and "next check" come from the server clock and are shown as relative times.
- **Update pinned a digest the registry later deletes**: Keel keeps running; the next check reports the tag's current target as usual.

## Requirements *(mandatory)*

### Functional Requirements

**What Keel is running**

- **FR-001**: Keel MUST know its version, source revision and build date from the build, and its image digest from the deployment. Missing values MUST be reported as unknown, never as placeholder text.
- **FR-002**: Owners and admins MUST see version, revision, digest and build date on the System tab. Other users MUST see the version only.
- **FR-003**: The owner's `keel_server` Ansible role MUST deploy Keel pinned to a digest, resolved from the configured tag at deploy time or given explicitly, MUST pass that digest to Keel, and MUST record the previous digest for rollback.

**Checking for updates**

- **FR-004**: Keel MUST NOT contact any registry or release site unless the update check is turned on in deployment configuration. It MUST be off by default.
- **FR-005**: With checks on, Keel MUST check shortly after start and then at a configurable interval, default 6 hours, minimum 1 hour. The watched image reference MUST be configurable, defaulting to the image Keel is published as.
- **FR-006**: A check MUST compare the digest the watched tag resolves to (for Keel's architecture) with the running digest, using requests that do not count as image pulls. When they differ, it MUST read the new image's version, revision and build date.
- **FR-007**: Owners MUST be able to start a check on demand, at most once a minute per instance.
- **FR-008**: Check results MUST be held in memory only. A restart MUST start from "not checked yet".
- **FR-009**: Invalid check configuration MUST stop Keel from starting, naming the setting.
- **FR-010**: A failed check MUST NOT report an update as available, MUST keep the last successful result, and MUST show the failure reason and next attempt to owners.
- **FR-011**: When the watched tag points to an older build, or a rebuild of the running revision, Keel MUST say so instead of reporting a plain update.

**Telling people**

- **FR-012**: When a check finds an update, connected owners MUST be told without reloading. Owners MUST see a dot on the System Settings navigation entry and an update card on the System tab.
- **FR-013**: The card MUST show current and available version, revision, build date and digest, the last check time, and a release-notes link when the new image carries a release version; otherwise it MUST say no release notes exist for untagged builds.
- **FR-014**: An owner MUST be able to dismiss the dot per available digest, remembered across reloads for that owner. Dismissing MUST NOT hide the card.
- **FR-015**: Admins who aren't owners MUST see at most a read-only line, without digest, dot or actions. Users MUST see nothing.

**Updating**

- **FR-016**: With a configured System workspace, an enrolled Keel host and `keel-server` allowed, owners MUST be able to update from the card through preview and passkey step-up.
- **FR-017**: The update MUST queue a `keel-server` run in the System workspace that pins the exact previewed digest. It MUST appear in Runs with the same review rules as any System run.
- **FR-018**: Only one update run MAY be queued or running at a time; a second request MUST show the existing run.
- **FR-019**: The update request, its outcome and the change of running digest MUST be audited with old and new digest. Refused requests MUST be audited.
- **FR-020**: Only owners, from a session, MAY request an update. Admins, users and API tokens MUST be refused.
- **FR-021**: Without the prerequisites of FR-016, the card MUST show manual steps (the digest to pin, commands to pull and restart) and name the missing step.
- **FR-022**: Keel MUST NOT update itself without an owner's confirmation. There is no automatic update mode.

### Key Entities

- **Running build**: version, revision, build date, image digest (any may be unknown). Fixed for the life of the process.
- **Update check result**: watched reference, when checked, outcome (up to date, update available, older build, rebuilt, can't tell, failed with reason), the available image's version, revision, build date and digest, next check time. In memory only.
- **Dot dismissal**: owner, dismissed digest. Per owner, so it survives reloads.
- **Update request**: the owner, the previewed old and new digest, the run it queued. Recorded through the audit log and the run itself.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With default configuration, a 24-hour run of Keel makes zero outbound connections to registries or release sites.
- **SC-002**: With checks on, an owner connected to Keel sees a published update within one check interval, or within 30 seconds of pressing Check now.
- **SC-003**: An owner can go from seeing the update card to Keel running the new image without opening a shell, in under 5 minutes plus the run's own duration.
- **SC-004**: 100% of update requests, refusals and digest changes appear in the audit log with old and new digest.
- **SC-005**: Over a week of checks every 6 hours, the update check counts zero image pulls against the registry's limits.
- **SC-006**: Admins and users never see a digest or an Update action, verified by end-to-end tests for each role.

## Assumptions

- Keel is published as one image, `mbround18/keel`, on Docker Hub, with the version, revision and build date in its standard image labels (the Dockerfile already sets them). Other registries work the same way given the image reference.
- Release notes live on the source repository's releases page and are found by the image's version. Images pushed without a release version have a revision but no release notes.
- Admins see a read-only line rather than nothing, so they can tell an owner; they get no button because an update changes the host Keel runs on (spec 060 makes the System workspace owner-only for the same reason).
- The default interval of 6 hours balances freshness against registry limits; checks that don't count as pulls make this safe.
- Results are not stored: after a restart, the first check comes shortly after start, which is enough.
- Dot dismissals are per owner and stored with the owner's preferences.
- Silent automatic updates (Watchtower-style) are deliberately out of scope for Keel: every change to Keel's host goes through preview, passkey and audit. Watchtower support for Depths, which manages many stacks, is a separate spec.
- `make keel-server` on the host stays the break-glass path and the way to roll back to a previous digest.

## Dependencies

- Spec 055: the audit log, passkey step-up and runs.
- Spec 058: the owner's `keel_server` role, its deferred compose restart, and `make keel-server` as break-glass.
- Spec 060: the owner-only System workspace and System tab (User Story 7), and the runner's replay of undelivered results.

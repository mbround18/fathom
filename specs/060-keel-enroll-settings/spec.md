# Feature Specification: Keel Enroll, System Settings, admin tokens, OIDC roles, System workspace and UI polish

**Feature Branch**: `060-keel-enroll-settings`

**Created**: 2026-10-28

**Status**: Implemented

**Input**: User description: "im the owner and no enroll hosts, can we add a screen from a button called Enroll that gives you the token and snippet? i also on the bottom of the left nav pane want a System Settings where you can set the snippet parts using like a good string formatter like %t for token or so so we can put our ansible stuff there and add optional instructions in markdown that also render on that enroll modal? and in the system settings offer the token management sub tab for revoke and roll kinda deal". Decided in conversation:
- spec first;
- the Markdown is rendered with a standard renderer with raw HTML off;
- the Tokens tab manages enrolment tokens and every user's API tokens.

**Revision input** (2026-10-28): "ensure the next person to sign in also doesn't get owner or better yet we read the roles from oidc to grant the proper roles via oidc roles? also ensure that you run impeccable skill on the ui so it all flows nicely its looking a bit unpolished". This adds User Story 5 (instance roles from the identity provider) and User Story 6 (a polish pass over Keel's UI).

**Revision input** (2026-10-28): "we should also on the system side if were the owner we should see the system ansible if configured via system settings allowing us to ansible our ansible on the keel server as needed". This adds User Story 7 (an owner-only System workspace for Keel's own host).

## Why

The instance owner signed in to a fresh Keel and could not enrol a host:
- Hosts offers only workspaces reached through a team grant, so an owner without one sees "You have no workspace yet" and no enrol form.
- Where the form does appear, the copy box shows `-e keel_enroll_token=<token>`, while the owner's Ansible repository reads the token from `KEEL_TOKEN` (`KEEL_TOKEN=… make keel-node`). The snippet is hard-coded and wrong for this repository, and every other repository will differ in its own way.
- A minted token can't be found or withdrawn afterwards. API tokens can be managed only by their own holder.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Enrol a host from an Enroll button (Priority: P1)

An instance owner opens Hosts and clicks **Enroll**. A dialog asks for the workspace, the host's inventory name and its profiles. The owner confirms with their passkey, and the dialog shows the one-time token, a snippet ready to paste on the host, and any instructions the admins wrote. They copy the snippet, run it on the host, and the host appears in the list.

**Why this priority**: This is the owner's blocker today. Without it no host can be enrolled through the UI.

**Independent Test**: Sign in as an owner who belongs to no team, on an instance with one workspace. Enrol a host from the dialog, run the copied snippet on a test host, and see the host connect. This works with the default template and no instructions set.

**Acceptance Scenarios**:

1. **Given** an instance owner or admin with no team grant, **When** they open Hosts, **Then** the workspace picker lists every workspace on the instance and the **Enroll** button is available.
2. **Given** a user who may enrol hosts in at least one workspace, **When** they click **Enroll**, **Then** a dialog opens with the workspace (defaulting to the one selected on the page), the inventory name, and the profiles (defaulting to `base`).
3. **Given** a filled dialog, **When** the user confirms, **Then** they are asked to verify with their passkey, as for any enrolment today. Only after that does the dialog show the token, the snippet built from the instance's template, and the instructions, if any are set.
4. **Given** the token is shown, **When** the user clicks Copy on the snippet or on the token, **Then** exactly that text is on the clipboard, and the dialog says the token will not be shown again, when it expires, and that it works once.
5. **Given** the token is shown, **When** the user closes the dialog, **Then** they are asked to confirm, because the token can't be shown again. After closing, it appears nowhere in the UI.
6. **Given** a user who may not enrol hosts in any workspace, **When** they open Hosts, **Then** there is no **Enroll** button.
7. **Given** no template has been set, **When** a token is shown, **Then** the snippet is the default Ansible one, which works unchanged with `make keel-node`.

---

### User Story 2 - Shape the snippet and instructions in System Settings (Priority: P2)

An instance admin opens **System Settings**, at the bottom of the left navigation, and edits the enrolment snippet template on the **Enrollment** tab. Placeholders such as `%t` (the token) and `%n` (the inventory name) mark where values go, and a live preview shows the result with sample values. The admin also writes optional Markdown instructions and previews them as they will appear. They save, confirm with their passkey, and the next enrolment uses the new snippet and instructions.

**Why this priority**: The fixed snippet suits one repository. Admins need to fit it to their own playbooks and tell enrollers what to do around it. Story 1 works without this, using the default template.

**Independent Test**: As an admin, set a template that uses every placeholder and add instructions containing a list, a code block and a link, then save. Enrol a host and check that the snippet has every value substituted and the instructions render as previewed. A non-admin can't see System Settings, and the server refuses their attempts to read or change these settings.

**Acceptance Scenarios**:

1. **Given** an instance owner or admin, **When** they look at the left navigation, **Then** **System Settings** is pinned at its bottom, below the sections. Other users don't see it.
2. **Given** the Enrollment tab, **When** the admin types in the template, **Then** the preview updates as they type, filling the placeholders with sample values and a dummy token.
3. **Given** a template containing an unknown placeholder (for example `%x`) or lacking `%t`, **When** the admin tries to save, **Then** saving is refused with a message naming the problem, and the stored template is unchanged.
4. **Given** instructions written in Markdown, **When** the admin switches to the preview, or a token is shown in the Enroll dialog, **Then** headings, lists, emphasis, tables, code blocks and links render. Raw HTML in the text is displayed as text and never interpreted.
5. **Given** a valid template and instructions, **When** the admin saves, **Then** they confirm with their passkey, the change is recorded in the audit log with who made it and the before and after text, and the next token shown anywhere uses it.
6. **Given** a template that has been changed, **When** the admin chooses to reset it, **Then** the default Ansible template is restored. Saving that is a normal, audited save.
7. **Given** two admins editing at once, **When** the second one saves after the first, **Then** the second save is refused as out of date, and they can reload to see the current text.

---

### User Story 3 - Revoke and roll enrolment tokens (Priority: P2)

On the **Tokens** tab, an instance admin sees every enrolment token on the instance: its workspace, host name, profiles, state, who created it, when, and when it expires. They revoke a token that leaked or was minted by mistake. They roll one that expired before the host was set up, which withdraws it and mints a new one for the same workspace, host and profiles. The new token is shown once, with the snippet and instructions, exactly as in the Enroll dialog.

**Why this priority**: Without this, a minted token stays valid until it expires and can't be withdrawn early. A token that expired too soon means filling in the Enroll dialog again.

**Independent Test**: Mint two tokens, revoke one and roll the other. The revoked token is refused by the server. The rolled one's old value is refused and its new value enrols the host. The list and the audit log show each step, and no token value appears in either.

**Acceptance Scenarios**:

1. **Given** enrolment tokens in several workspaces, **When** an admin opens the Tokens tab, **Then** all of them are listed, newest first. Each shows its state:
   - pending: unused and unexpired;
   - used: it enrolled a host;
   - revoked;
   - burned: spent by a failed attempt;
   - expired: unused past its expiry.

   The list can be filtered by state and workspace.
2. **Given** a pending token, **When** the admin revokes it and confirms with their passkey, **Then** it is revoked at once, any later attempt to enrol with it is refused, and the action is recorded in the audit log.
3. **Given** a token that is pending, expired, revoked or burned, **When** the admin rolls it and confirms with their passkey, **Then** the old token is withdrawn if it wasn't already, and a new token is minted for the same workspace, inventory name and profiles, with a fresh expiry. The new token is shown once, with the snippet and instructions. The audit log records both the withdrawal and the new token, linked to each other.
4. **Given** a used token, **When** the admin views it, **Then** neither Revoke nor Roll is offered. Its row names the host it enrolled.
5. **Given** any listing or audit entry, **When** it is inspected, **Then** no token value appears in it, only the token's identity and its metadata.

---

### User Story 4 - Revoke any user's API tokens (Priority: P3)

Below the enrolment tokens, an instance admin sees every API token on the instance: whose it is, its name, workspace, allowed actions, when it was created and last used, and when it expires. They revoke one, for example the token of someone who left or a token that leaked.

**Why this priority**: Today a token's holder is the only one who can find it in the UI. Removing a user already revokes their tokens, so this matters most when one token leaked but its holder should keep the rest.

**Independent Test**: Create API tokens as two different users. As an admin, see both on the Tokens tab, revoke one with a passkey, and check that it no longer authenticates while the other still does.

**Acceptance Scenarios**:

1. **Given** API tokens held by several users, **When** an admin opens the Tokens tab, **Then** all of them are listed with their holder and state (active, revoked, expired), filterable by holder and state.
2. **Given** an active API token, **When** the admin revokes it and confirms with their passkey, **Then** it stops authenticating at once, its holder sees it as revoked in their own list, and the audit log records who revoked it.
3. **Given** an API token, **When** the admin views it, **Then** nothing on the page can show its secret again, and there is no roll action. The holder creates a new token themselves.

---

### User Story 5 - Instance roles come from the identity provider (Priority: P1)

The owner manages people in Keycloak, not in Keel. Members of a Keycloak role or group named, for example, `keel-owners` sign in to Keel as owners, members of `keel-admins` as admins, and members of `keel-users` as users. Anyone in none of them is turned away at sign-in. Removing someone from a group changes their role the next time they sign in, and their current sessions end within the session limit.

Today the first account on an empty instance becomes owner and everyone after that becomes a user. That stays the rule when no mapping is configured, and two people signing in at once can never both become the first owner.

**Why this priority**: Without a mapping, anyone the realm authenticates gets a Keel account. Roles then drift between the identity provider and Keel, and an owner has to promote each admin by hand.

**Independent Test**: Configure a mapping against a test realm with three users, one in each group, plus a fourth in no group. Each of the first three signs in with the mapped role and the fourth is refused. Move one user between groups: their role changes at their next sign-in, the change is audited, and their old sessions stop working.

**Acceptance Scenarios**:

1. **Given** no mapping is configured and the instance has no users, **When** two people complete sign-in at the same moment, **Then** exactly one of them becomes owner and the other becomes a user.
2. **Given** no mapping is configured and the instance has users, **When** a new person signs in, **Then** they become a user, never an owner or admin.
3. **Given** a mapping is configured, **When** a person signs in through the mapped provider, **Then** their instance role is the highest one their roles or groups map to (owner, then admin, then user), on first sign-in and on every sign-in after.
4. **Given** a mapping that requires a matching role or group, **When** a person matching none of them signs in, **Then** sign-in is refused with a message telling them to ask an administrator. No account is created, and the refusal is audited.
5. **Given** a person's mapped role changes in the identity provider, **When** they next sign in, **Then** their role changes, the change is audited with the reason "identity provider", and sessions issued under the old role stop working.
6. **Given** a mapping is configured, **When** an admin opens Users, **Then** roles of people managed by the mapping are shown as managed by the identity provider and can't be changed in Keel.
7. **Given** a mapping is configured, **When** an admin opens System Settings, **Then** they see the mapping read-only: the provider, the claim read, the values for each role, and whether a match is required. The mapping can't be changed there.
8. **Given** a mapped sign-in would leave the instance with no owner, **When** it is processed, **Then** the last owner keeps the owner role, the sign-in otherwise succeeds, and the audit log and System Settings both warn that the identity provider names no owner.

---

### User Story 6 - Keel's UI reads as one finished product (Priority: P2)

The screens this spec adds, and the shell and sections around them, share one visual language: spacing, type scale, empty and loading states, dialogs, tables, buttons, copy blocks and error lines all look and behave the same way from page to page. The owner described the current UI as "a bit unpolished".

**Why this priority**: New screens built in the current style would add to the unevenness. A pass over the whole UI now costs less than one per feature later.

**Independent Test**: A design review of every section at phone and desktop widths, in light and dark themes, finds no inconsistent spacing, type, control or state treatment, and no layout that breaks at 360 px. The section's end-to-end tests still pass.

**Acceptance Scenarios**:

1. **Given** any section, **When** it is empty, loading or failed, **Then** it uses the shared state treatment, with the same placement, wording style and actions.
2. **Given** any table, dialog, form or copy block, **When** it appears in two sections, **Then** it looks and behaves the same in both.
3. **Given** a 360 px wide screen, **When** each section and dialog is opened, **Then** nothing scrolls horizontally except deliberately scrollable tables, and every action can be reached.
4. **Given** the new navigation, **When** it is viewed, **Then** System Settings sits visibly apart at the foot of the navigation, and the current section is clearly marked.

---

### User Story 7 - Owners manage the Keel server's own host from Keel (Priority: P3)

The owner keeps the Keel server's host in the same Ansible repository as everything else (`make keel-server`). In System Settings, a **System** tab lets the owner point Keel at that repository and enrol the Keel server's host into a reserved **System** workspace. From then on the owner can preview and run the `keel-server` and `base` profiles against Keel's own host from Keel, with the same reviews, audit and run history as any other run. Admins and users never see the System workspace.

**Why this priority**: Today the Keel host is the one machine that still needs a shell and `make keel-server` for every change. This closes that gap. Everything else in this spec works without it, and `make keel-server` on the host stays the way to repair it.

**Independent Test**: On a test instance, configure the System tab with a test repository and enrol the instance's own host. Preview a `keel-server` run, run it, and see it finish. When the run restarts Keel, the restart comes after the run has reported, and the result is shown once Keel is back. An admin who isn't an owner can't see the System workspace in any list, and the server refuses their direct requests for it.

**Acceptance Scenarios**:

1. **Given** an owner and an unconfigured System tab, **When** they open it, **Then** they see what it is for, and the steps: choose the repository and branch, choose which profiles may run (`keel-server` and `base` by default), then enrol the Keel host. Nothing about the System workspace appears anywhere else until those steps are done.
2. **Given** the repository is chosen, **When** the owner saves, **Then** the save needs a passkey and is audited. The System workspace is created if it doesn't exist, with the owner role as its only members. The repository is bound to it the same way any workspace binds a repository.
3. **Given** a configured System workspace, **When** the owner clicks Enroll on the System tab, **Then** the Enroll dialog (User Story 1) opens with the System workspace and the allowed profiles preselected, and the snippet and instructions shown as usual.
4. **Given** the Keel host is enrolled, **When** the owner opens the System tab, **Then** they see the host's state, its last runs and a Run action for each allowed profile. Each run goes through preview and, if the workspace requires it, review, like any other run.
5. **Given** a run that restarts Keel itself, **When** it reaches the restart, **Then** the run reports its outcome first and the restart happens afterwards. When Keel comes back, the run shows as finished with that outcome. If Keel does not come back within the expected time, the host-side runner records it, and the System tab shows it once Keel is up again.
6. **Given** an admin or user who isn't an owner, **When** they list workspaces, hosts, runs or audit entries, or call the System workspace's actions directly, **Then** the System workspace and everything in it are absent or refused.
7. **Given** a configured System workspace, **When** the owner removes the configuration, **Then** after a passkey confirmation the Keel host is revoked and the workspace is archived, never deleted, keeping its run history and audit. This is audited.

---

### Edge Cases

- **No workspaces exist yet**: the Enroll button is visible to admins, and the dialog says a workspace must be created first, with a link to where that's done. It never offers an empty picker.
- **`%s` used but the runner address is not configured**: saving is refused, saying which setting is missing. A template stored before the address went away renders the gap visibly as `<runner address not set>`, not as an empty string, and the settings page warns about it.
- **Literal percent signs** (for example `printf '%s'` inside the snippet) are written `%%`. A `%` followed by any character not on the placeholder list is refused on save.
- **A very long template or very long instructions**: capped at 4 KB for the template and 20 KB for the instructions, enforced when saving and stated beside the fields.
- **Links in the instructions** open in a new tab and can't reach back to the opening page. Images aren't loaded from the network; they appear as links, so the instructions can't track who opens the dialog.
- **The token expires while the dialog is open**: the dialog keeps showing what was minted but marks it expired, and offers Roll if the user may roll.
- **The user loses the right to enrol** (for example a team grant is removed) between opening the dialog and confirming: confirmation is refused, and no token is minted.
- **Revoking a token while a host is using it**: if the host's first contact has already been accepted, the host stays enrolled, and the token is shown as used, not revoked. Revoking an enrolled host's access remains the host's own revoke action.
- **The settings cannot be read** (storage failure): enrolment still works with the default template, the dialog says custom instructions could not be loaded, and the failure is logged. Showing a token is never blocked by the settings.
- **A user who is not an admin calls the settings or token-management actions directly**: the server refuses them. Hiding the navigation entry is not the protection.
- **The identity provider omits the roles claim**, for example because the mapper isn't set up: with a match required, sign-in is refused and the audit log names the missing claim. Without that requirement, the person gets the user role. A claim that is missing is never read as "no change".
- **Several providers are configured** (OIDC plus GitHub or Discord): the mapping applies only to the provider it names. People from other providers get the user role, or are refused when the mapping requires a match for all sign-ins.
- **An existing owner signs in after the mapping is turned on but isn't in the owner group**: they are demoted like anyone else, unless they are the last owner (User Story 5, scenario 8).
- **The mapping is configured wrongly**, for example with an unknown role name or an empty value list: Keel refuses to start and says which setting is wrong. It does not fall back to first-user-is-owner.
- **The Keel host is already enrolled in another workspace**: enrolling it into the System workspace is refused until it's revoked there, so one host never takes runs from two workspaces.
- **The last owner is demoted while the System workspace exists**: the instance always keeps an owner (FR-028), so the System workspace always has a member who can see it.

## Requirements *(mandatory)*

### Functional Requirements

**Enrol from Hosts**

- **FR-001**: Hosts MUST offer an **Enroll** button to every user who may enrol hosts in at least one workspace, and to no one else.
- **FR-002**: Instance owners and admins MUST be able to choose any workspace on the instance wherever Hosts asks for one, whether or not they hold a team grant to it.
- **FR-003**: The Enroll dialog MUST collect the workspace, the inventory name and the profiles, with the same checks enrolment applies today. It MUST require the passkey confirmation enrolment already requires before any token is minted.
- **FR-004**: After minting, the dialog MUST show:
  - the token;
  - the snippet built from the instance's template, with copy buttons for both;
  - the expiry, and the fact that the token works once and will not be shown again;
  - the instance's instructions, rendered, when set.
- **FR-005**: The dialog MUST ask for confirmation before it closes while a token is on screen. After closing, the token MUST NOT be retrievable anywhere.
- **FR-006**: The inline enrol form and token box on Hosts MUST be replaced by the button and dialog, so there is one place that shows tokens.

**System Settings**

- **FR-007**: A **System Settings** entry MUST be pinned at the bottom of the left navigation for instance owners and admins only. It MUST open a page with an **Enrollment** tab and a **Tokens** tab.
- **FR-008**: Admins MUST be able to edit one instance-wide snippet template, using these placeholders:

  | Placeholder | Replaced with |
  |---|---|
  | `%t` | the token |
  | `%n` | the inventory name |
  | `%p` | the profiles, comma-separated |
  | `%w` | the workspace's slug |
  | `%s` | the address runners connect to (host:port) |
  | `%u` | Keel's public URL |
  | `%e` | the expiry, as a UTC timestamp |
  | `%%` | a literal `%` |

- **FR-009**: Saving MUST be refused when the template:
  - contains `%` followed by anything not in FR-008;
  - lacks `%t`;
  - uses `%s` while the runner address is not configured;
  - exceeds 4 KB.

  Saving the instructions MUST be refused when they exceed 20 KB.
- **FR-010**: Placeholders MUST be replaced in a single pass. Text that comes from a substituted value MUST NOT itself be read as a placeholder.
- **FR-011**: The default template MUST produce, for the owner's Ansible repository, a snippet that works unchanged:

  ```text
  read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t
  make keel-node
  ```

  Admins MUST be able to restore the default at any time.
- **FR-012**: The Enrollment tab MUST preview both the snippet, filled with sample values and a dummy token that can't be mistaken for a real one, and the rendered instructions, updating as the admin edits.
- **FR-013**: Instructions MUST be rendered as Markdown, including tables, task lists and code blocks. Raw HTML MUST be shown as text and never interpreted. Links MUST open in a new tab with no access to the opener. Images MUST NOT be fetched.
- **FR-014**: Saving settings MUST require passkey confirmation. It MUST be recorded in the audit log with the actor and the before and after text. It MUST be refused if the settings changed since the editor loaded them.
- **FR-015**: Reading and changing settings, and every token-management action, MUST be refused by the server for anyone who is not an instance owner or admin.

**Tokens**

- **FR-016**: The Tokens tab MUST list every enrolment token on the instance. Each row MUST show the workspace, the inventory name, the profiles, the state (pending, used, revoked, burned, expired), the creator, the creation time and the expiry, plus the enrolled host for used tokens. The list MUST be filterable by state and workspace.
- **FR-017**: Admins MUST be able to revoke a pending enrolment token. A revoked token MUST be refused from that moment, with the same response as any other invalid token.
- **FR-018**: Admins MUST be able to roll an enrolment token that is pending, expired, revoked or burned. Rolling MUST withdraw the old token and mint its replacement for the same workspace, inventory name and profiles as one step: if either fails, neither happens. The replacement MUST be shown once, in the same form as FR-004.
- **FR-019**: Used tokens MUST offer neither revoke nor roll.
- **FR-020**: The Tokens tab MUST list every API token on the instance, with its holder, name, workspace, actions, creation time, last use, expiry and state, filterable by holder and state. Admins MUST be able to revoke any of them.
- **FR-021**: Revoke and roll MUST require passkey confirmation. Each MUST be recorded in the audit log with the actor and the token's identity and metadata. A roll's two entries MUST be linked.
- **FR-022**: Token values MUST NOT be stored, logged, put in the audit log, or shown after the moment they are minted, for both enrolment and API tokens.

**Roles from the identity provider**

- **FR-023**: Without a mapping, only the first account on an empty instance MUST become owner. This MUST hold when sign-ins happen at the same moment. Every later account MUST become a user.
- **FR-024**: The deployment MUST be able to configure a role mapping for the OIDC provider. The mapping consists of:
  - the claim holding roles or groups, including nested claims such as Keycloak's realm roles;
  - the values that grant owner, admin and user;
  - whether a match is required to sign in at all.

  It is set in the deployment's configuration, alongside the provider's other settings, not in the UI.
- **FR-025**: With a mapping, the instance role MUST be recomputed from the provider's claims at every sign-in through that provider. The highest matching role wins.
- **FR-026**: With a match required, a person matching none of the values MUST be refused at sign-in, without an account being created, and the refusal MUST be audited.
- **FR-027**: A role change made by the mapping MUST be audited with the reason "identity provider". It MUST end the sessions issued under the old role.
- **FR-028**: The mapping MUST never leave the instance without an owner (User Story 5, scenario 8).
- **FR-029**: Roles of people managed by the mapping MUST be read-only on the Users page. System Settings MUST show the active mapping read-only.
- **FR-030**: An invalid mapping MUST stop Keel from starting, with an error naming the setting.

**Polish**

- **FR-031**: Every section, dialog and the navigation MUST use one shared set of treatments for spacing, type, tables, dialogs, buttons, copy blocks, and empty, loading and error states.
- **FR-032**: Every section and dialog MUST work at 360 px width with no horizontal page scroll, and in both light and dark themes.

**System workspace**

- **FR-033**: System Settings MUST have a **System** tab, visible only to instance owners, to configure the repository, branch and allowed profiles (default `keel-server`, `base`) for managing Keel's own host.
- **FR-034**: Saving that configuration MUST create, or reuse, one reserved System workspace, whose only members are instance owners, and MUST bind the repository to it through the existing binding mechanism. The save MUST require a passkey and MUST be audited.
- **FR-035**: The System workspace and everything in it (hosts, runs, reviews, audit entries) MUST be invisible to everyone but instance owners, in every list and on direct request. This MUST be enforced by the server.
- **FR-036**: Runs in the System workspace MUST use the same preview, review, run and audit flow as any other workspace. Nothing about them MUST be exempt.
- **FR-037**: A run against Keel's own host that restarts Keel MUST record its outcome before the restart. Its final state MUST be shown once Keel is back, including when Keel did not come back in time.
- **FR-038**: Removing the configuration MUST revoke the Keel host and archive the System workspace, keeping its history. It MUST require a passkey and MUST be audited.
- **FR-039**: When the System tab isn't configured, the System workspace MUST NOT exist or appear anywhere.

### Key Entities

- **Enrolment settings**: one per instance. Holds the snippet template, the Markdown instructions, when they were last changed, by whom, and a version used to detect concurrent edits. Absent until first saved; the default template applies until then.
- **Enrolment token**: exists today. Gains a revoked state, the admin who revoked it, and a link from a rolled token to the token that replaced it.
- **API token**: exists today. Gains an admin-wide view; nothing about it changes.
- **Audit entry**: exists today. Gains entries for:
  - settings saved;
  - enrolment token revoked, and enrolment token rolled;
  - API token revoked by an admin;
  - sign-in refused by the mapping, and role changed by the identity provider.
- **Role mapping**: deployment configuration, not stored data. It names one provider, the claim to read, the values for owner, admin and user, and whether a match is required.
- **User**: exists today. Gains a mark saying their role is managed by the identity provider.
- **System configuration**: the repository, branch and allowed profiles for Keel's own host, set by an owner. Saving and removing it are audited.
- **System workspace**: one reserved workspace, created when the system configuration is first saved. Only instance owners are members and can see it. It is archived, never deleted, when the configuration is removed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An instance owner with no team grant can go from opening Hosts to a copied, working snippet in under one minute.
- **SC-002**: The snippet from the default template, pasted unchanged on a fresh Debian host with the owner's Ansible repository, enrols the host on the first try.
- **SC-003**: After an admin saves a template, 100% of tokens minted from then on show the new snippet, and none show the old one.
- **SC-004**: A revoked or rolled-away enrolment token is refused on 100% of later attempts, and a revoked API token is refused on its very next request.
- **SC-005**: Searching the audit log, the server's logs and every UI listing after a test run finds no token value.
- **SC-006**: Every settings save, revoke and roll in a test run has exactly one matching audit entry naming the actor; a roll has two linked entries.
- **SC-007**: A user who is not an instance owner or admin cannot see System Settings, and gets a refusal for each of its actions when calling them directly.
- **SC-008**: With a mapping and a required match, 100% of people outside the mapped groups are refused at sign-in. 100% of people inside them get exactly the role their highest group maps to.
- **SC-009**: Over 100 simultaneous first sign-ins against an empty instance with no mapping, exactly one owner exists afterwards.
- **SC-010**: Within one sign-in of a group change in the identity provider, the person's Keel role matches it.
- **SC-011**: A design review across every section, at 360 px and desktop widths and in both themes, finds no inconsistent treatment and no horizontal page scroll.
- **SC-012**: An owner can change the Keel server's host configuration from Keel, from preview to a finished run, without opening a shell on the host.
- **SC-013**: A non-owner, checking every list and direct request, finds no trace of the System workspace.

## Assumptions

- Settings are instance-wide, not per workspace. Per-workspace templates can come later if workspaces turn out to use different repositories.
- "May enrol hosts" keeps today's meaning, with instance owners and admins added for every workspace.
- Enrolment tokens keep today's 15-minute lifetime. A rolled token gets a fresh 15 minutes.
- Instructions are written by trusted admins but still rendered safely, since they are shown to every enroller.
- The runner address for `%s` comes from Keel's existing configuration. This feature adds no new setting for it.
- Only one-time enrolment tokens are covered. Join tokens (055 `contracts/join.md`) are not built yet; when they are, they join the Tokens tab.
- The Markdown renderer is a standard, maintained library with raw HTML off, loaded only when instructions are shown. The owner approved adding it.
- The role mapping lives in deployment configuration, not in the UI. An admin who could edit it in the UI could make themselves owner, while deployment configuration is reviewed in the infrastructure repository. The owner's Ansible role gains matching variables.
- Role changes take effect at the person's next sign-in. Keel does not poll the identity provider between sign-ins; the session lifetime bounds how long an old role can last.
- The polish pass (User Story 6) is done with the project's `impeccable` design skill, across all of Keel's sections, not only the new screens. It changes presentation, not behaviour, so existing end-to-end tests keep passing.
- The System workspace is for owners only, not admins, because a run there changes the host Keel itself runs on.
- The restart-after-report behaviour relies on the deferred compose restart that spec 058 gives the owner's `keel_server` role. Repositories without it are still allowed, but their runs may show as interrupted when Keel restarts during them.
- `make keel-server` on the host stays the break-glass path (spec 058). Keel is never the only way to repair its own host.

## Dependencies

- Spec 055: host enrolment, API tokens, passkey step-up and the audit log, all of which this extends.
- The owner's Ansible repository: `make keel-node` reads `KEEL_TOKEN`, which the default template targets.

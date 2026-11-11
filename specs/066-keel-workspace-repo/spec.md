# Feature Specification: Keel workspaces, repository catalog and hosts

**Feature Branch**: `066-keel-workspace-repo`

**Created**: 2026-11-11

**Status**: Draft

**Input**: User description: "In Keel, on the Settings page I want to configure the Ansible repo
per workspace. Workspaces need a proper concept: a workspace dropdown like Fathom's namespace
picker, with create. Then set up the repo, and Keel syncs the roles, playbooks and the rest into
the frontend for us to select. It must be sops aware and let us add new hosts to the workspace."
Decisions taken with the requester (2026-11-11):

- Picking a discovered playbook that no job declares **drafts a job** for `.keel/jobs.yaml`.
  Runs still start only from declared jobs, so the repository keeps deciding what can run.
- Adding a host **enrolls its runner and opens a pull request** that adds it to the repository's
  inventory. Git stays the source of truth for inventory.
- Scope: every story below.

**Builds on**: spec 055 (bindings, sync, manifest, inventories, enrolment), spec 058 (the
operator's Ansible repository and its manifest), spec 060 (System Settings, `ListWorkspaces`),
spec 063 (navigation and guided empty states), spec 065 (the shared UI kit).

## Why

Keel already has workspaces. They have a table, grants and a create command on Teams. What it
lacks is a way to *work in* one:

- Each of five pages draws its own workspace picker and forgets the choice when you leave.
- Creating a workspace is buried in Teams, and a new workspace has no project, so the first
  binding needs a detour through Teams to make one.
- The repository is a "binding" on a project, reached from Pipelines. Nothing says "this
  workspace's Ansible repository", and nothing shows what is in it beyond the jobs the manifest
  declares.
- To write `.keel/jobs.yaml` by hand you need to know every role a playbook reaches. The
  operator's repository has a lint script just to check that list.
- Secrets in the repository are sops files. Keel cannot see whether a host can decrypt the
  secrets its job reads until the run fails on the host.
- To add a host you enroll the runner in Keel, then separately edit `inventory/hosts.yml` in git.
  If you forget the second step, every job on that host fails with "no such host in inventory".

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Work in one workspace at a time (Priority: P1)

The shell's top bar has a workspace switcher, like the namespace picker in Fathom. It lists the
workspaces the person can see and marks the one they are in. Runs, Reviews, Bindings, Receivers,
Inventory, Hosts and the new Repository page all show the chosen workspace. The choice survives
reloads and travels in the address, so a shared link opens the same workspace. Owners and admins
also see "Create workspace…" in the switcher. A new workspace is created with a default project
and becomes the current one.

**Why this priority**: every other story happens "in a workspace". Without one current workspace,
each new page needs another picker.

**Independent Test**: as an owner with two workspaces, switch to the second from the top bar,
reload, open Hosts and then Runs, and confirm both show the second workspace. Create a third from
the switcher and confirm it becomes current and has a project named `default`.

**Acceptance Scenarios**:

1. **Given** a person with grants in two workspaces, **When** they pick the second in the
   switcher, **Then** every workspace page shows it, and the address carries `?ws=<slug>`.
2. **Given** a link with `?ws=<slug>` for a workspace the person can see, **When** they open it,
   **Then** that workspace is current. **Given** one they cannot see, **Then** Keel opens their
   remembered or first workspace and says the linked one isn't available to them.
3. **Given** an owner or admin, **When** they choose "Create workspace…", enter a name, accept
   the derived slug and confirm, **Then** the workspace exists with a `default` project, the
   switcher selects it, and the audit log records both.
4. **Given** a person who may not create workspaces, **Then** the switcher has no create entry.
5. **Given** a person who can see no workspace, **Then** the switcher shows the guided empty
   state from spec 063 instead of an empty list.
6. **Given** a workspace that is archived while it is current, **Then** the switcher moves to
   another workspace and says why.

---

### User Story 2 - Configure the workspace's repository in Settings (Priority: P1)

Settings has a **Workspace** tab for the current workspace. A workspace admin sees it even if
they are not an instance owner or admin. The tab's Repository section shows the workspace's
*primary repository*: the binding that the catalog, sops checks and add-host pull requests use.

With no repository, the section offers a guided form for URL, branch, transport, credential
(created inline), pinned known host for SSH, and poll interval. Confirming it creates a binding
on the workspace's default project, marks it primary and syncs it once. With a repository, the
section shows the last sync, head commit, manifest status and **Sync now**. It links to the
binding's full page for signed mode, receivers and credential grants. The same tab sets the
workspace's **forge connection** (provider, API base, token credential), which Story 5 uses to
open pull requests.

**Why this priority**: it is the entry point the requester named, and Stories 3 to 5 read the
primary repository.

**Independent Test**: in a workspace with no binding, configure a local test repository from
Settings → Workspace. Confirm that a binding exists on the `default` project, is marked primary,
and has synced. Change the primary to a second binding and confirm the catalog follows it.

**Acceptance Scenarios**:

1. **Given** a workspace admin with no instance role, **When** they open Settings, **Then** they
   see the Workspace tab only. They do not see Enrollment, Tokens, Identity or System.
2. **Given** a workspace with no repository, **When** an admin submits the form and confirms the
   preview, **Then** Keel creates the binding and marks it primary in one change, syncs it, and
   shows the result or the sync error.
3. **Given** a workspace with several bindings, **When** an admin picks another as primary,
   **Then** the catalog and sops view switch to it on their next read.
4. **Given** a forge connection with a token that cannot write to the repository, **When** the
   admin tests it, **Then** Keel says so and names the permission the token lacks.
5. Removing the primary binding leaves the workspace without a primary repository. It never
   promotes another binding silently.

---

### User Story 3 - See what is in the repository and draft jobs from it (Priority: P1)

A new **Repository** page shows what Keel found at the primary repository's head commit. Keel
rebuilds this catalog on every sync. It has:

- **Playbooks**: every playbook file, with its plays' `hosts:` patterns, the roles it reaches
  (through `roles:`, `import_role`, `include_role` and `import_playbook`, followed to a fixed
  depth), and the job or jobs that declare it.
- **Roles**: every role, with its description from `meta/main.yml`, its default variable names
  (never their values), and the playbooks that reach it.
- **Inventory**: each inventory's groups, child groups and hosts, and which hosts are enrolled
  runners in this workspace.
- **Jobs**: the manifest's jobs, as on the binding page, cross-linked to the above.

Selecting a declared job offers **Run**, which opens the existing start-run flow. Selecting a
playbook that no job declares offers **Draft job**. Keel writes a `.keel/jobs.yaml` entry for
it, with these paths derived from what the playbook reaches:

- the playbook itself;
- `roles/<role>/**` for each role it reaches;
- every playbook it imports;
- the group vars of the groups its plays target;
- `ansible.cfg`, `requirements.yml` and `inventory/group_vars/all.yml`;
- the sops files those group vars name (Story 4).

The admin can copy the draft or, with a forge connection, open it as a pull request (Story 5).
The draft never starts a run.

**Why this priority**: this is the "sync the roles and playbooks into the frontend for us to
select" the requester asked for. It also replaces hand-maintained path lists.

**Independent Test**: sync a fixture repository modelled on the operator's (playbooks, roles,
inventory with child groups, a manifest declaring some playbooks). Confirm the Repository page
lists every playbook and role. Confirm an undeclared playbook's draft lists exactly the expected
paths, and a declared job's Run opens the start-run dialog with that job chosen.

**Acceptance Scenarios**:

1. **Given** a synced primary repository, **When** a viewer opens Repository, **Then** they see
   the playbooks, roles, inventory and jobs at the head commit, with the commit and sync time.
2. **Given** a playbook that imports another, **Then** its reached roles include the imported
   playbook's roles, and the draft's paths include the imported playbook.
3. **Given** a role reached only through a templated name (`include_role: name: "{{ x }}"`),
   **Then** the playbook shows "reaches roles Keel cannot name", and the draft carries a comment
   saying so instead of guessing.
4. **Given** a sync that fails, **Then** the page shows the last good catalog marked stale, with
   the error.
5. **Given** a repository larger than the catalog limits (files, bytes per file, total bytes),
   **Then** the catalog is built from what fits and says what it skipped.
6. **Given** a viewer without Operate, **Then** Run is not offered. Draft job is offered to anyone
   who can see the page, because it only produces text.

---

### User Story 4 - Know who can decrypt each secret before a run fails (Priority: P1)

The Repository page has a **Secrets** tab. Keel parses `.sops.yaml` (its creation rules: path
regex and age or PGP recipients) and each sops-encrypted file's *unencrypted* metadata: the
recipients, last modified time, and which keys are encrypted. Keel never holds a sops or age
private key, never decrypts, and never stores or shows an encrypted value.

Each runner reports its host's age public key, when the host has one, alongside the facts it
already reports. With that, Keel shows for each secret file:

- the recipients it is encrypted to;
- whether that matches the creation rule (if not, `sops updatekeys` is pending);
- the jobs that read it, from their paths and the vars files that name it;
- which enrolled hosts those jobs run on can decrypt it, and which cannot.

Starting a run whose job reads a sops file that a target host cannot decrypt shows a warning in
the preview. It is a warning, not a refusal: the host decides, and its key may be newer than its
last report.

**Why this priority**: the requester asked for sops awareness. A host missing from a file's
recipients is the most common reason a first run on a new host fails.

**Independent Test**: sync the fixture with a `.sops.yaml` and two encrypted files, one
re-encrypted and one not. Connect two fake runners, one whose age key is a recipient. Confirm the
Secrets tab shows the pending `updatekeys`, the job-to-file links and the per-host decrypt state.
Confirm a run preview on the other host carries the warning.

**Acceptance Scenarios**:

1. **Given** an encrypted file whose recipients differ from its creation rule's, **Then** the
   file shows "recipients out of date" with the added and missing keys and the command to fix it.
2. **Given** a host whose age key is not among a file's recipients and a job on that host that
   reads the file, **Then** the file lists the host under "cannot decrypt", and a run preview
   for that host warns.
3. **Given** a host that has not reported an age key, **Then** it shows as "age key unknown",
   not as "cannot decrypt".
4. **Given** any encrypted file, **Then** no response, log line, audit row or stored catalog
   contains its ciphertext or any value. Only key names, recipients and timestamps appear.
5. **Given** a file that is named like a sops file but is malformed, **Then** the Secrets tab
   lists it with the parse problem, and the rest of the catalog still builds.

---

### User Story 5 - Add a host to the workspace (Priority: P2)

From Hosts (and from a group on Repository → Inventory) an operator chooses **Add host**:

1. They give the host's inventory name, pick one or more inventory groups from the primary
   repository's catalog, and pick profiles, limited to the ones the manifest declares.
2. Keel shows what will happen:
   - an enrolment token, with the existing enrolment command;
   - the change to the inventory file, as a diff that keeps every comment and the file's layout;
   - when the groups' jobs read sops files, a note that this host's age key must be added to
     `.sops.yaml` and those files re-encrypted.
3. On confirm, Keel issues the token and, with a forge connection, opens a pull request on a new
   branch with the inventory change. Without one, it shows the patch to apply by hand.
4. Once the runner enrolls and reports its age key, the host's page offers **Propose sops
   recipient**: a second pull request that adds the key to the matching `.sops.yaml` creation
   rules. Its description says that someone who can already decrypt must run
   `sops updatekeys <file>` for each listed file. Keel cannot re-encrypt.

The host appears on Hosts as *pending inventory* until the head commit's inventory lists it,
then as ready.

**Why this priority**: it completes the onboarding loop, but enrolment already works on its own,
so this is P2.

**Independent Test**: against a fake forge, add host `newbox` to group `compose_hosts`. Confirm
the pull request's single commit changes `inventory/hosts.yml` by inserting `newbox:` under that
group's `hosts:` with every comment intact. Confirm the token works, and the host moves from
pending inventory to ready after a sync of a head that contains it.

**Acceptance Scenarios**:

1. **Given** a group with existing hosts and comments, **When** a host is added, **Then** the
   diff inserts exactly one line, in that group's `hosts:` block and in its sort position, and
   changes nothing else.
2. **Given** a group whose `hosts:` block does not exist yet, **Then** Keel adds the block under
   the group, indented like its siblings.
3. **Given** a name already in the inventory, **Then** Keel refuses with `host_exists` and names
   the group it is in.
4. **Given** a forge that refuses the push or pull request, **Then** the enrolment token is still
   issued, the error is shown with the patch to apply by hand, and the audit log records both.
5. **Given** a primary repository in signed mode, **Then** the pull request's description says
   its commit is unsigned and must be signed or squashed by a trusted signer before Keel applies
   it, and Keel does not treat the pull request itself as trusted.
6. **Given** an operator without the Operate capability, **Then** Add host is not offered, and the
   server refuses the command.

---

### Edge Cases

- **A workspace has bindings but no primary.** This happens to workspaces from before this spec.
  The migration marks the oldest active binding primary. If there is none, the Workspace tab says
  the workspace has no repository yet.
- **Two admins set different primaries at once.** Last write wins. Each change is audited, and
  the tab shows who set the current one.
- **A catalog from an old commit.** The catalog records the commit it was built at. Run previews
  and drafts use the catalog only for hints. The job set still comes from the manifest at the
  run's own commit.
- **YAML Keel cannot parse.** It is listed as a catalog problem with file and line. It never
  fails the sync, and never blocks runs.
- **Symlinks and paths outside the repository.** They are treated as the manifest validator
  treats them (spec 055 `manifest.rs`): never followed outside the tree.
- **An inventory that is a directory or a script.** The catalog lists the groups and hosts it can
  read statically (YAML and INI files). A dynamic inventory script shows as "dynamic, not read".
  Add host is offered only for YAML inventories.
- **A forge token with too much access.** Keel uses it only to create a branch, commit to that
  branch and open a pull request. It never writes to the bound branch directly. A forge that
  needs a direct push is not supported.
- **A runner older than this spec.** It reports no age key, and its host shows "age key unknown".
- **A switcher selection for a workspace that disappears** while a page is open. The page's next
  read gets `not_found`, and the switcher moves as in Story 1, scenario 6.

## Requirements *(mandatory)*

### Functional Requirements

**Workspaces (Story 1)**

- **FR-001**: The shell MUST show a workspace switcher on every signed-in page that has
  workspace-scoped content. Its options MUST be what `ListWorkspaces` returns, without archived
  workspaces.
- **FR-002**: The current workspace MUST come from the address (`?ws=<slug>`) first, then the last
  one this browser chose, then the first visible one. Pages MUST read it from one shared store,
  and the per-page pickers MUST be removed.
- **FR-003**: Owners and admins MUST be able to create a workspace from the switcher, through the
  existing preview and confirm flow.
- **FR-004**: Creating a workspace MUST also create a project with slug `default`, in the same
  transaction and audit entry. Existing workspaces with no project MUST get one from the
  migration.

**Repository settings (Story 2)**

- **FR-005**: Settings MUST have a Workspace tab, shown to owners, admins and workspace admins of
  the current workspace. Workspace admins without an instance role MUST see only that tab.
- **FR-006**: A workspace MUST have at most one primary repository binding. Setting it MUST need
  workspace admin, and MUST be previewed, confirmed and audited.
- **FR-007**: The Workspace tab MUST be able to create a binding on the default project and mark
  it primary in one confirmed change, then sync it.
- **FR-008**: A workspace MAY have one forge connection: provider (`forgejo`, `gitea` or
  `github`), API base URL, and a `forge_token` credential. Keel MUST be able to test it without
  writing anything.

**Catalog (Story 3)**

- **FR-009**: Each successful sync of a primary binding MUST build a catalog of the head commit
  and store it with the commit. A failed build MUST keep the previous catalog, marked stale.
- **FR-010**: The catalog MUST be built by static reading only. Keel MUST NOT run Ansible, Jinja
  or any repository code to build it.
- **FR-011**: The catalog MUST list playbooks, roles, inventories (groups, children, hosts) and
  jobs, with the links between them described in Story 3. Templated names it cannot resolve
  MUST be reported, not guessed.
- **FR-012**: The catalog build MUST respect fixed limits: at most 5,000 files examined, 1 MiB per
  file, 32 MiB read in total, and an import depth of 8. It MUST report what it skipped.
- **FR-013**: Keel MUST produce a job draft for any catalog playbook, as manifest YAML, from the
  derivation in Story 3. Drafting MUST NOT change the job set or start a run.
- **FR-014**: Reading the catalog MUST need the View capability on the workspace.

**Sops (Story 4)**

- **FR-015**: The catalog MUST parse `.sops.yaml` creation rules and the `sops` metadata of files
  matching them or named `*.sops.*`. It MUST NOT keep encrypted values. Parsing MUST discard them
  before the catalog is stored.
- **FR-016**: The runner MUST report its host's age public key in `Hello`, from a configurable
  public-key file, when present. Keel MUST store it on the host and show it.
- **FR-017**: For each secret file, Keel MUST show recipients, creation-rule drift, reading jobs,
  and per-host decrypt state (can, cannot, unknown).
- **FR-018**: A run preview MUST warn, without refusing, when a target host cannot decrypt a sops
  file that the job reads.

**Adding hosts (Story 5)**

- **FR-019**: Operators MUST be able to add a host: issue an enrolment token (as today), and
  propose an inventory change that adds the host to chosen groups of a YAML inventory in the
  primary repository.
- **FR-020**: The inventory change MUST be a minimal textual edit that keeps comments, key order,
  indentation and line endings. It MUST be computed by a pure function and checked by re-parsing
  the result.
- **FR-021**: With a forge connection, Keel MUST open the change as a pull request: a new branch
  named `keel/add-host-<name>-<short id>`, one commit, never a direct write to the bound branch.
  Without one, Keel MUST return the change as a unified diff.
- **FR-022**: Keel MUST be able to propose adding an enrolled host's age key to `.sops.yaml` the
  same way. It MUST say that re-encryption is a manual step.
- **FR-023**: A host MUST show *pending inventory* until the primary repository's head inventory
  lists its inventory name.
- **FR-024**: Proposing a job draft as a pull request MUST use the same forge path and the same
  rules.

**Cross-cutting**

- **FR-025**: Every new mutating command MUST follow Keel's preview and confirm pattern, be
  capability-checked at preview and again at confirm, and be audited without secrets.
- **FR-026**: Forge writes MUST be behind the runtime flag `KEEL_FORGE_WRITES`, on by default.
  With it off, Keel MUST still produce diffs and drafts.
- **FR-027**: The guides in `docs/guides/keel` MUST cover workspaces, the repository settings, the
  catalog, sops awareness and adding a host.

### Key Entities

- **Workspace**: existing. Gains `primary_binding` and an optional forge connection.
- **Catalog**: for one binding at one commit, the playbooks, roles, inventories, secret files,
  problems and skipped counts. It is one stored document per binding, replaced on each build.
- **Secret file**: a path, its creation rule, its recipients (age and PGP), its encrypted key
  names, its last modified time and its drift. Never its values.
- **Host**: existing. Gains `age_recipient` and a derived inventory status (`listed` or
  `pending_inventory`).
- **Forge connection**: provider, API base and token credential, per workspace.
- **Repository change**: a proposed edit (add host, add sops recipient, add job), its diff and,
  once opened, its pull request URL and branch. It is audited, not stored as its own table.

## Success Criteria *(mandatory)*

- **SC-001**: Switching workspace takes one action from any workspace page, and every workspace
  page agrees on the current workspace after a reload.
- **SC-002**: From a fresh instance, an owner can create a workspace, configure its repository
  and see its catalog without leaving Settings and the Repository page, and without visiting
  Teams.
- **SC-003**: For the operator's real repository (`ansible/`), every job draft for a declared
  playbook derives a path list that `scripts/keel-manifest-check.py` accepts.
- **SC-004**: No catalog, response, log line or audit row produced from a repository with sops
  files contains an encrypted value. This is checked by a test that searches all of them for the
  fixture's ciphertext.
- **SC-005**: Adding a host to a commented inventory produces a one-line diff in 100% of the
  fixture cases.
- **SC-006**: A run on a host that cannot decrypt its job's secrets is warned about before it
  starts in 100% of the fixture cases.

## Assumptions

- The primary repository is an Ansible repository laid out like the operator's: `playbooks/`,
  `roles/`, a YAML inventory. Other layouts still sync and run. Their catalog is just thinner.
- Forgejo and Gitea share an API. GitHub's contents and pulls API is close enough for one port
  with two adapters. GitLab is out of scope, and its users get the diff.
- Age is the sops backend that matters. PGP recipients are parsed and shown, but only age keys
  are matched against hosts, because hosts report only an age key.
- Runner and server ship together. Because the new `Hello` field is optional, an old runner works
  against a new server. A new runner against an old server is not supported.

## Out of Scope

- Running a playbook that no job declares.
- Decrypting, re-encrypting or editing secret values.
- Editing arbitrary repository files from Keel. Only the three changes above are proposed.
- GitLab merge requests.
- Merging pull requests from Keel.

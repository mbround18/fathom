# Research: Keel workspaces, repository catalog and hosts

## R1. The current workspace lives in the address and one store

**Decision**: one zustand store, `web/src/app/workspace-store.ts`. It resolves the current
workspace in this order: `?ws=<slug>` in the address, then `localStorage["keel.workspace"]`, then
the first visible workspace. The switcher sits in `NavShell`'s top bar, before the account menu.
Pages call `useCurrentWorkspace()` and stop drawing their own picker. `WorkspaceSelect` stays as
the switcher's inner control. `GrantWorkspacePicker` is deleted.

**Why**: the address makes links shareable (FR-002). `localStorage` makes a reload stay put.
Reading every option from `ListWorkspaces` instead of `me.grants` (which is what
`GrantWorkspacePicker` reads) fixes the existing gap where an owner with no grants saw nothing.
`hosts/workspaces.ts` already documents that gap.

**Rejected**: a path prefix (`/w/<slug>/runs`). It would rewrite every route and every saved
link, and it buys nothing over a query parameter.

**Look**: Fathom's `ui/src/components/NamespacePicker.tsx`, a compact select with an "all" row.
Keel has no "all workspaces" view, so it gets no "all" row. It does get a separator and
"Create workspace…".

## R2. A workspace always has a `default` project

**Decision**: `PreviewWorkspaceCreate` inserts a `default` project in the same transaction.
Migration 0014 adds one to every workspace that has none. Bindings keep hanging off projects.
Settings creates the primary binding on `default`.

**Why**: bindings need a project, and today the first binding means a detour through Teams.
Making projects optional would touch every binding query. One default row touches none of them.

## R3. "Primary repository" is a column on workspaces

**Decision**: `workspaces.primary_binding uuid references repo_bindings(id) on delete set null`.
The migration backfills it with each workspace's oldest active binding. A new command,
`PreviewWorkspacePrimary { workspace, binding: Option<Uuid> }`, sets or clears it. A binding
created from Settings is created and made primary in one confirmed change
(`PreviewWorkspaceRepository`).

**Why**: at most one primary per workspace falls out of the column. A boolean on `repo_bindings`
would need a cross-table unique constraint, since bindings belong to projects.

## R4. The catalog is a pure keel-core module over a file port

**Decision**: add `crates/keel-core/src/catalog/`:

- `mod.rs`: the `Catalog` types and the `build(files: &impl RepoFiles, manifest: Option<&Manifest>,
  limits: Limits) -> Catalog` entry point;
- `playbook.rs`: plays, `hosts:` patterns, `roles:`, `import_role`, `include_role`,
  `import_playbook`, `vars_files`;
- `role.rs`: `meta/main.yml` description, default variable *names*, `dependencies`;
- `inventory.rs`: reuses `keel-core/src/inventory.rs` parsing for YAML, plus a small INI reader;
- `sops.rs`: `.sops.yaml` rules and the metadata of encrypted files;
- `draft.rs`: the job draft (R6).

`RepoFiles` has `read(path) -> Option<Vec<u8>>` and `list(dir) -> Option<Vec<Entry>>`. The
server implements it on `keel_git::Mirror` at a commit, the same way `sync.rs` already does
`InventoryFiles`. Tests implement it on an in-memory map.

**Why**: this follows the agnostic-core rule. The core never sees git, so the catalog is
testable without a repository, and nothing executes repository content (FR-010).

**What counts as a playbook**: a `.yml` or `.yaml` file, outside `roles/`, `inventory/`,
`group_vars/`, `host_vars/`, `.github/`, `.forgejo/`, `.keel/` and `molecule/`. Its top level must
be a sequence of mappings, each with `hosts` or `import_playbook`. Everything else is ignored, so
`requirements.yml` and compose files never show as playbooks.

**What counts as a role**: a directory `roles/<name>/` that has `tasks/main.yml` or
`tasks/main.yaml`. Paths from `ansible.cfg`'s `roles_path` are added when it names relative
directories inside the repository.

**Role reach**: playbook roles and role `meta/main.yml` dependencies are followed. In task files,
`include_role` and `import_role` with a literal `name` are followed, and so are `include_tasks`
and `import_tasks` with a literal path. Any value containing `{{` is reported as unresolved.
Collection roles (`ns.coll.role`) are listed as external and not followed.

## R5. Sops: metadata only, values dropped at parse time

**Decision**: a file is treated as sops when it matches a creation rule's `path_regex`, or is
named `*.sops.{yaml,yml,json,env}`. The YAML or JSON is parsed. Only the top-level `sops` mapping
and the *key paths* of the other mappings are kept. Every string value of the form
`ENC[AES256_GCM,...]` is dropped when it is reached, and so is any other leaf value. dotenv files
keep only key names. From `sops` Keel keeps:

- `age[].recipient`;
- `pgp[].fp`;
- `lastmodified`;
- `encrypted_regex`, `encrypted_suffix`, `unencrypted_suffix`;
- `version`.

The MAC and the encrypted data keys (`enc`) are dropped.

`.sops.yaml` is parsed for `creation_rules[].{path_regex, age, pgp, key_groups[].{age, pgp}}`.
`age` may be a comma-separated string or a list. Rules are matched first-match, as sops does.
`path_regex` is compiled with the `regex` crate, which is already in the lockfile through
keel-core's dependency tree. It is size-limited, and a pattern that fails to compile is a catalog
problem.

**Drift**: the set of age keys in the file's metadata is compared with the set the rule names.

**Who reads which file**: a job reads a sops file when one of these is true:

- one of the job's `paths` globs matches the file;
- a `vars_files` entry or `community.sops.load_vars` `file:` in the playbook names it literally;
- a group-vars file of a group the job targets names it literally.

**Why**: the spec forbids storing ciphertext (FR-015, SC-004). Dropping values during the parse
means no later bug can leak them. The SC-004 test greps everything produced for the fixture's
ciphertext.

## R6. Job drafts derive paths the way `keel-manifest-check.py` does

**Decision**: `draft.rs` emits one job entry:

- `playbook`;
- `profiles`, the manifest profiles whose names match the playbook's stem, otherwise `[]` with a
  comment;
- `inventory`, the manifest's only inventory, or the first one, with a comment;
- `paths`, sorted;
- `timeout: 30m`.

The paths are the operator's documented derivation (ansible repo, `.keel/jobs.yaml` header):

- `ansible.cfg`, `requirements.yml` and `inventory/group_vars/all.yml`, when they exist;
- the playbook and every playbook it imports;
- `roles/<r>/**` for every reached role;
- `<inventory dir>/group_vars/<g>.yml` for every inventory group that a play's `hosts:` pattern
  names directly, plus the parents of those groups that have a vars file;
- `playbook_dir`-relative `files/` paths named literally;
- the sops files R5 attributes to it.

`inventory/hosts.yml` is never added: adding a host must not trigger every job, which is the
operator's rule.

**Check**: SC-003 runs the drafts for the operator's repository through a test that compares them
with the declared jobs' `paths`.

## R7. Runners report an age public key

**Decision**:

- `keel-exec` `Frame::Hello` gains `age_recipient: Option<String>`, with `#[serde(default,
  skip_serializing_if = "Option::is_none")]`.
- The runner reads it from the file set by `keel-runner`'s config `age_recipient_file` (default
  `/etc/sops/age/recipient.txt`). It takes the first line, trimmed, and must match
  `^age1[02-9ac-hj-np-z]{58}$`. Otherwise it sends `None`.
- The server stores it in `hosts.age_recipient`.
- In the operator's Ansible repository, `roles/sops` writes that file, mode `0644`, from
  `age-keygen -y`. It is a public key, so the unprivileged runner can read it, and the private key
  stays `0440 root:keel-exec`.

**Why**: the runner's network-facing process cannot read the private key, and must not.

## R8. Forge port with Forgejo/Gitea and GitHub adapters

**Decision**:

- In keel-core `ports.rs`, add `trait Forge { fn probe(); fn open_change(ChangeRequest) ->
  OpenedChange }`.
- `ChangeRequest` is `{ base_branch, new_branch, title, body, files: Vec<FileEdit { path,
  expected_blob_sha, new_content }> }`.
- Adapters live in a new crate, `crates/keel-forge`, on `reqwest`, which is already a workspace
  dependency through keel-git.
- **Forgejo and Gitea**: `POST /repos/{o}/{r}/contents` (`ChangeFiles`, with `new_branch` and
  `sha` per file, so a concurrent edit to the file fails instead of being overwritten), then `POST
  /repos/{o}/{r}/pulls`.
- **GitHub**: `GET /repos/{o}/{r}/git/ref/heads/{base}`, then `POST /repos/{o}/{r}/git/refs`, `PUT
  /repos/{o}/{r}/contents/{path}` with `branch` and `sha`, and `POST /repos/{o}/{r}/pulls`.
- `probe` reads the repository and checks `permissions.push`.
- Owner and repository names are parsed from the binding URL. The API base defaults to
  `https://<host>/api/v1` for Forgejo and Gitea, and to `https://api.github.com` for GitHub.

**Why**: neither path needs a git push. gix has no supported push, and the server must not shell
out to git. The `sha` precondition means Keel never overwrites a change it has not seen.

**Flag**: `KEEL_FORGE_WRITES`, read at runtime from config, default on (FR-026).

## R9. Minimal inventory edits are textual

**Decision**: `keel-core/src/inventory_edit.rs` has `add_host(text, group, host) ->
Result<Edit, EditError>`. It works in five steps:

1. Parse with the existing YAML inventory reader, to find the group and refuse `host_exists`.
2. Scan the text for the group's key line at its indentation, walking `all: children:` nesting.
3. Find or create its `hosts:` child.
4. Insert `<indent>host:` in sorted position among the existing host keys. Trailing comments stay
   with the host they follow.
5. Re-parse the result and check that the inventory now equals the old one plus the host.

Line endings and a final newline are kept. Flow-style mappings (`hosts: {a: , b: }`) are refused
with `unsupported_layout` and fall back to "apply by hand".

`add_sops_recipient(text, rule_index, recipient)` does the same for `.sops.yaml`. It appends to
the rule's age list, in either list or comma-string form.

`add_job(text, name, draft_yaml)` appends under `jobs:` with a blank line.

**Why**: the operator's inventory is mostly comments (FR-020, SC-005). Re-serialising YAML would
drop them.

## R10. Settings stays one page; the Workspace tab follows the current workspace

**Decision**: add `'workspace'` to `SettingsTab`. `visibleTabs(role, me, workspace)` returns
`['workspace', ...]` for owners and admins, and `['workspace']` for a workspace admin of the
current workspace. The nav's Settings entry shows when either is true. `DEFAULT_TAB` becomes
`workspace`. The page title becomes "Settings", because it is no longer only system settings.

## R11. Repository is a new Pipelines section

**Decision**: add a section `/repository`, with a book icon, between Bindings and Receivers. It
has tabs: Playbooks, Roles, Inventory, Secrets, Jobs. Each tab is a lazily loaded chunk, and
selecting an item opens a side detail pane. Add host is on Hosts and on an Inventory group's
detail.

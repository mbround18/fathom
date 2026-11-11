# UI contract: Keel workspaces, repository catalog and hosts

Built from the shared UI kit (`@companion/ui/kit`, spec 065). There are no new primitives unless
noted.

## Shell: workspace switcher (Story 1)

- It sits in the top bar, left of the account menu, and carries `data-testid="workspace-switcher"`.
- It is a dropdown menu. The trigger shows the current workspace name, then its slug muted. The
  items are the workspaces in slug order, with a check on the current one. Owners and admins also
  get a separator and "Create workspace…" (`data-action="create-workspace"`).
- Create opens a dialog with Name and Slug. The slug is derived from the name, editable, and
  validated as `^[a-z0-9][a-z0-9-]{0,62}$`. Submitting goes through the governance `ChangeDialog`
  (preview and confirm). On success the new workspace becomes current.
- With no workspaces, the trigger reads "No workspace", and the menu holds the spec 063 guided
  empty state.
- At 360 px the trigger shows only the icon and slug, truncated.
- The address carries `?ws=<slug>`. Changing the workspace replaces it, and does not push a new
  history entry.

## Settings → Workspace (Story 2)

The tab's sections are titled with the current workspace name:

1. **Repository**: either the guided form (URL, branch, transport, credential select with "New
   credential…", known host when SSH, poll interval) or a summary: URL, branch, head (`ShortCopy`),
   last sync (`Time`), manifest state, Sync now, "Open binding", and "Change primary…" (a select
   of the workspace's bindings).
2. **Forge connection**: provider, API base (with its default as placeholder), and a token
   credential select limited to `forge_token`. It has Test (shows the `ForgeProbe` result) and
   Remove.
3. **Danger zone**: archive the workspace, linking to the existing flow.

## Repository page (Story 3, Story 4)

- `PageHeader`: "Repository", with the URL, branch and head commit and the sync time. A stale
  catalog gets a warning `StatusBadge` with the error.
- Tabs: **Playbooks** · **Roles** · **Inventory** · **Secrets** · **Jobs**. Each is a dense
  list, with a side detail pane on selection. The selection is in the address (`?item=`), so it
  can be linked.
- Playbook detail:
  - plays and their host patterns;
  - reached roles, as links;
  - imports;
  - unresolved notes;
  - jobs that declare it.
  - Actions: **Run** (per declared job; Operate) and **Draft job**. Draft job opens a dialog with
    the YAML in a read-only code block, Copy, and, with a forge connection, **Propose as pull
    request** (name field, then preview and confirm).
- Role detail: description, defaults (names), dependencies, reached by.
- Inventory: groups as a tree, with host chips. An enrolled host's chip links to it. A group's
  detail has **Add host here** (Operate).
- Secrets: the rules file and rules, then files. Each file row shows:
  - its recipients count;
  - a drift badge ("recipients out of date");
  - the reading jobs;
  - decrypt chips per host: ok, cannot, or unknown.

  The file detail shows the `sops updatekeys <path>` command (`ShortCopy`).
- Empty states, using the spec 063 pattern:
  - no primary repository, with an action to configure it in Settings;
  - not synced yet;
  - nothing found.

## Hosts (Story 5)

- An **Add host** button (Operate) opens a three-step dialog:
  1. Name, groups (a multi-select from the catalog's YAML inventory) and profiles (from the
     manifest).
  2. Review: the enrolment command, the inventory diff, and sops notes.
  3. Result: the token, plus the pull request link, or the patch with Copy.
- The host row shows an inventory status badge: "pending inventory" (warn) or nothing when listed.
- Host detail shows the age key (`Fingerprint`, or "not reported"), and **Propose sops recipient**
  (workspace admin) when the key isn't in the matching rules.

## Test hooks

- `data-testid`: `workspace-switcher`, `repo-tab-<name>`, `catalog-item`, `draft-yaml`,
  `add-host`, `change-outcome`.
- `data-decrypt="ok|cannot|unknown"` on decrypt chips.
- `data-inventory-status` on host rows.

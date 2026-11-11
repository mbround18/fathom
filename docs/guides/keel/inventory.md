# Inventory

An inventory is the Ansible inventory a job runs against: the hosts and the groups they are in.
Keel keeps each inventory as a series of revisions. An inventory either comes from a file in a
bound repository, or is written and edited in Keel itself. This guide explains the difference and
how inventories meet the hosts you enrolled.

## Managed and repository inventory

| | Repository inventory | Managed inventory |
|---|---|---|
| Where it lives | A file in a bound repository, declared in `.keel/jobs.yaml` | In Keel |
| **Source** column | **repo** | **managed** |
| How it changes | Push to the repository; each sync that changes it makes a new revision | Edit it in Keel; each save makes a new revision |
| Editable in Keel | No | Yes, with the Workspace admin role |

A repository inventory is read-only in Keel. To change it, change the file in the repository.

Both are plain Ansible inventories, in YAML or INI. Neither is a list of enrolled hosts: the
**Hosts** column counts the hosts the newest revision lists, enrolled or not.

### How inventory meets your hosts

A run is pinned to its commit and to the inventory's newest revision when it is made. It reaches
the hosts in that revision that:

- the job's `limit` selects;
- are enrolled in the workspace and not revoked, under the same name (the **Inventory name** given
  when the host was enrolled);
- serve one of the job's profiles.

A host in the inventory that never enrolled is simply not reached. So is an enrolled host whose
inventory name doesn't appear. See [Enroll a host](hosts.md#enroll-a-host).

If the newest revision is invalid, runs against it are refused with `inventory_invalid`. Keel does
not fall back to an older revision.

### Bring in your repository's inventory

There is no separate import step. Either:

- declare the inventory under `inventories` in `.keel/jobs.yaml` and push, so the next sync reads
  it (see [Bindings and credentials](bindings.md#bind-a-repository)); or
- paste its YAML into a new managed inventory, if you want to edit it in Keel from then on.

### Create a managed inventory

1. Open **Infrastructure → Inventory** in the workspace you want; see [Switch workspace](workspaces.md#switch-workspace).
2. Choose **New inventory**. The **New managed inventory** form opens.
3. Enter a **Name** and write the inventory in YAML.
4. Optionally, add a variable schema as JSON. Keel then checks the inventory's variables against
   it.
5. Choose **Preview inventory**, check it, and choose **Confirm**.

Keel warns about a plain-text variable that looks like a secret. Keep secrets in a credential
instead (see [Credentials](bindings.md#credentials)).

### Edit, compare and restore

Select an inventory to open it.

- **Revision N (newest)** shows the inventory. For an active managed inventory you can edit it and
  choose **Save revision**, or **Discard changes**. Keel refuses to save while there are errors, and a change to layout only doesn't make a new
  revision.
- The variable schema can be changed with **Save schema** or removed with **Remove schema**.
- **Revisions** lists every revision, who made it and when. Revisions never change once made.
  **View** opens an old one, and **Restore** makes a new revision with its content.
- **Compare revisions** shows what changed between two revisions: pick **From** and **To**, then
  choose **Compare**.
- **Hosts** lists the hosts in the newest revision and their groups.

If someone else saved first, Keel refuses your save with `stale_base`. Reload and make the change
again.

**Deprecate** retires an inventory. A deprecated inventory accepts no new runs.

# Workspaces and the primary repository

A workspace keeps a set of hosts, repositories, inventories and runs apart from everything else.
Keel always shows one workspace at a time: the one named in the switcher at the top of every page.
This guide covers switching and creating workspaces, giving a workspace its Ansible repository, and
connecting the forge that hosts that repository, so Keel can open pull requests for you.

## Switch workspace

The workspace switcher sits in the top bar, left of your account menu. It shows the current
workspace's name and slug.

1. Open the switcher.
2. Choose a workspace. The ones you can see are listed by slug, with a check on the current one.

Every page follows the switcher: Runs, Reviews, Bindings, Receivers, Repository, Inventory and
Hosts all show the current workspace. The address carries it as `?ws=<slug>`, so a link you share
opens the same workspace for the person who follows it, as long as they can see it. Keel also
remembers your last workspace, so a reload stays put.

If the workspace in a link is archived or you can't see it, Keel opens your first workspace
instead.

## Create a workspace

Creating a workspace needs the instance owner or admin role.

1. Open the switcher and choose **Create workspace…**.
2. Enter a **Name**. Keel suggests a **Slug** from it: lowercase letters, digits and dashes,
   starting with a letter or digit. Change it if you like. The slug is what appears in links.
3. Keel shows what it will do. Choose **Confirm**.

The new workspace becomes current. It comes with a project called `default`, which is where the
primary repository's binding lives. Give people access to it from **Teams**; see
[Getting started](getting-started.md#give-people-access).

## Set the primary repository

Each workspace has one primary repository: the Ansible repository Keel reads playbooks, roles,
inventories and sops files from, and the one **Repository** shows. Setting it needs the Workspace
admin role.

1. Open **Settings → Workspace**. The sections are titled with the current workspace's name.
2. Under **Repository**, enter:
   - **Repository URL**, for example `https://git.example.com/org/ansible.git`;
   - **Branch**, usually `main`;
   - **Transport**, HTTPS or SSH. For SSH, also paste the forge's **Known host line**;
   - **Credential**: choose one, or **New credential…** to add a token or key here. Choose
     **None (public)** for a public repository over HTTPS;
   - **Poll every (seconds)**, optional. Leave it blank to sync only on a webhook or **Sync now**.
3. Choose **Preview repository**, then **Confirm**.

Keel creates the binding on the workspace's `default` project, makes it primary and syncs it.
Afterwards the section shows the URL, branch, head commit and last sync, with **Sync now** and
**Open binding**.

A workspace that already has bindings can make any of them primary with **Change primary…**.
Removing the primary binding leaves the workspace without one until you choose another.

Runs still come only from the jobs the repository declares in `.keel/jobs.yaml`. See
[Bindings and credentials](bindings.md).

## Connect a forge

With a forge connection, Keel can open pull requests against the primary repository: to add a new
host to the inventory, to add a host's age key to `.sops.yaml`, or to add a drafted job. Without
one, Keel shows you the change as a patch to apply yourself. Connecting a forge needs the Workspace
admin role.

1. Create a token at your forge that can write to the repository:
   - **Forgejo or Gitea**: an access token with the `write:repository` scope.
   - **GitHub**: a fine-grained token on the repository with **Contents** and **Pull requests** set
     to read and write.
2. Under **Forge connection**, choose the **Provider**.
3. **API base** defaults to `https://<host>/api/v1` for Forgejo and Gitea and to
   `https://api.github.com` for GitHub. Change it only if your forge serves its API elsewhere.
4. Choose the **Credential**, or **New credential…** to store the token as a `forge_token`.
5. Choose **Preview connection**, then **Confirm**.
6. Choose **Test**. Keel asks the forge what the token may do, and says whether it can push.

Keel only ever writes to a new branch and opens a pull request; it never pushes to your branch and
never merges. It also checks that each file it edits is still the version it read, so it never
overwrites a change it hasn't seen.

To stop Keel opening pull requests, choose **Remove**. Instance operators can also turn forge
writes off for the whole instance with `KEEL_FORGE_WRITES=false`; changes are then always shown as
a patch.

## Archive a workspace

Archiving needs the instance owner or admin role. Under **Danger zone**, choose **Archive
workspace**, then **Confirm**. An archived workspace leaves the switcher; links to it open your
first workspace instead.

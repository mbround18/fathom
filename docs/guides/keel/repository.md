# The repository catalog

**Pipelines → Repository** shows what is in the current workspace's primary repository:
playbooks, roles, inventories, sops-encrypted files and declared jobs. Keel reads these files on
every sync of the primary repository. It never runs anything in the repository to build this view,
and it never decrypts a secret.

The page needs a primary repository. See
[Set the primary repository](workspaces.md#set-the-primary-repository).

## Read the page

The line under the title gives the commit the catalog was built from and when it was built. It can
also show:

- **stale**: the last build failed. Keel shows the last catalog that built, and says why the new one
  failed.
- **N problems**: files Keel could not read, such as a playbook that isn't valid YAML. Hover to list
  them.
- **N files skipped**: the repository is larger than Keel reads. Hover for the reason.

Before the first sync finishes, the page says **Not synced yet** and updates when the sync is
done.

The page has five tabs: **Playbooks**, **Roles**, **Inventory**, **Secrets** and **Jobs**. Choosing
a row opens its details beside the list. The address keeps the tab and the row, so you can share a
link to either.

## Playbooks

A playbook is a YAML file outside `roles/`, `inventory/`, `group_vars/`, `host_vars/`, `.keel/`,
`molecule/` and forge folders, whose top level is a list of plays. Other YAML files, such as
`requirements.yml` or compose files, are left out.

A playbook's details show its **Plays** and the hosts each one targets, the **Roles** it reaches,
its **Imports**, its **Vars files** and the jobs that run it. A playbook marked **unresolved**
names a role or file with a template, such as `{{ role_name }}`. Keel can't follow those, so they
are listed under **Not followed**.

## Draft a job

Runs only come from the jobs declared in the repository's `.keel/jobs.yaml`. To declare one for a
playbook:

1. Open the playbook and choose **Draft job**.
2. Keel shows a **Job entry** for `.keel/jobs.yaml`. It fills in:
   - `playbook`;
   - `profiles`, the manifest profiles named like the playbook, if any;
   - `inventory`;
   - `paths`: the playbook, what it imports, the roles it reaches, the group vars of the groups it
     targets, and the sops files it reads. A push that changes one of these paths triggers the job.
     The inventory's host list is never included, so adding a host doesn't trigger every job.
   - `timeout`.

   The **Notes** under the entry say what Keel guessed. Check them.
3. Copy the entry into `.keel/jobs.yaml` yourself, or, with a forge connection, enter a **Job name**
   and choose **Propose as pull request**. Keel shows what it will do; choose **Confirm**.

Keel opens the pull request on a new branch. The job exists once it is merged and Keel has synced.

## Roles

A role is a `roles/<name>/` directory with `tasks/main.yml`. Its details show its **Description**,
the names of its **Defaults** (not their values), what it **Depends on**, and the playbooks that
reach it under **Reached by**. Roles from a collection, such as `community.general.x`, are listed
but not read.

## Inventory

The Inventory tab lists the repository's inventories and their groups, with each group's
**Children**, **Hosts** and **Group vars**. A dynamic inventory only shows its path, because its
groups are only known when it runs.

A group's details offer **Add host here**, which opens [Add host](hosts.md#add-a-host) with that
group already chosen.

## Secrets

Keel reads the sops metadata of each encrypted file: who it is encrypted for and the names of its
keys. It drops every encrypted value as it reads the file, so no value is ever stored or shown.

The Secrets tab lists the `.sops.yaml` **Creation rules** and each encrypted file with:

- its **Recipients**, the age and PGP keys it is encrypted for;
- the jobs that read it, under **Read by**;
- the enrolled hosts those jobs run on, each marked **can decrypt**, **cannot decrypt** or **no
  key reported**.

A host can decrypt a file when its age key is one of the file's recipients. A host reports its key
when its runner connects: see [Hosts and age keys](hosts.md#age-keys).

### When a file is out of date

A file is **Out of date** when its recipients don't match the rule in `.sops.yaml` that covers it,
usually because a key was added to the rule after the file was encrypted. The details list the
**Missing recipients** and **Extra recipients**, and the command that fixes it:

```sh
sops updatekeys secrets/example.sops.yaml
```

Run it from a machine that can already decrypt the file, then commit the result. Keel never
re-encrypts a file, because that needs a key that can decrypt it.

## Jobs

The Jobs tab lists the jobs in `.keel/jobs.yaml`, with their playbook, profiles and trigger paths.
A job marked **blocked** can't run, and its details say why. Choose **Run** to start one; see
[Runs and approval](runs.md).

If the job reads sops files that some of its hosts can't decrypt, the run's preview says so before
you confirm.

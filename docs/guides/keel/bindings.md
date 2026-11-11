# Bindings and credentials

A binding ties one branch of a repository to a project in a workspace. When the branch moves, Keel
fetches it and starts runs for the jobs whose files changed. Keel reads a private repository with a
credential you add first. This guide covers credentials, including what **Preview credential**
does, and binding a branch.

Everything on the Bindings page needs the Workspace admin role, except reading it and **Sync now**,
which an Operator can use.

## Credentials

A credential is a secret a binding uses to fetch the repository or to run its jobs. Keel stores
the value sealed, never shows it again, and lists only its fingerprint: a short code made from the
value, so you can tell two credentials apart without seeing them.

| Kind | Used for |
|---|---|
| **Git HTTPS token** | Fetching a repository over HTTPS |
| **Git SSH key** | Fetching a repository over SSH |
| **Vault password** | Ansible Vault, for jobs that use it |
| **Environment** | Environment variables for a job |
| **Secret vars** | Secret Ansible variables for a job |

To add one:

1. Open **Pipelines → Bindings** in the workspace you want; see [Switch workspace](workspaces.md#switch-workspace).
2. Under **Credentials**, in **New credential**, enter a **Name**.
3. Choose the **Kind** and paste the **Value**.
4. Choose **Preview credential**. This is the form's submit button, not a separate kind of
   credential: Keel shows what adding it will change, and nothing is saved yet.
5. Choose **Confirm**, and confirm with your passkey. Keel saves the value and clears the field.

The table lists each credential's **Name**, **Kind**, **Fingerprint**, and the bindings it is
**Granted to**.

A job can use a credential only if it is granted to the job's binding. Open the binding, choose the
credential under **Grant a credential**, and select **Grant**. A job whose manifest names a
credential that isn't granted shows as blocked with `credential_unavailable`. **Revoke** takes a
grant back.

Keel refuses to remove a credential a binding still fetches with.

## Bind a repository

Before you start, you need:

- the repository's URL and the branch to follow;
- a credential, if the repository is private (see [Credentials](#credentials));
- for SSH, the server's host key line, as in `known_hosts`;
- a project in the workspace for the binding to belong to. Create one under **Administration →
  Teams** if there is none.

1. Open **Pipelines → Bindings** and choose **New binding**.
2. Pick the **Project**. If the workspace has only one, Keel picks it for you. With none, the form
   links to Teams to create one.
3. Enter the **Repository URL** and the **Branch**. The branch is `main` unless you change it.
4. Choose the **Transport**, **HTTPS** or **SSH**.
5. Choose the **Credential**, or **None (public)**. Only credentials that match the transport are
   listed.
6. For SSH, paste the **Known host line**. Keel needs the server's host key pinned.
7. To have Keel check for changes on a timer as well as on webhooks, set **Poll every (seconds)**.
   Leave it blank to sync only when a webhook arrives.
8. Choose **Preview binding**. Keel fetches the branch once and shows the head it found and whether
   the manifest is valid.
9. Choose **Confirm**.

To have pushes sync the binding, bind a receiver to it. See
[Receivers](receivers.md#create-a-receiver).

### The manifest

Keel learns a repository's jobs from `.keel/jobs.yaml` on the bound branch. It declares the
profiles, the inventories and the jobs. Each job names its playbook, the profiles it runs on and
its inventory, and can list the credentials it needs and the `paths` whose changes start it. A job
without `paths` never starts on a push.

If the manifest is invalid, Keel keeps the jobs from the last valid one and shows the problems on
the binding, with the error `manifest_invalid`.

### The binding's page

Select a binding in the table to see it.

- **Last commit**, **Synced**, **Polling** (or "webhook only") and any **Error**.
- **Manifest problems**, if there are any.
- **Jobs at** the last commit: each job's **Playbook**, **Profiles**, **Inventory** and **State**.
  A job is **ready**, or **blocked** with the reason.
- The credentials granted to it.

Its buttons:

- **Sync now** fetches the branch straight away.
- **Disable** stops syncs and webhook deliveries for the binding. **Enable** turns them back on.
- **Remove** deletes the binding.

# Getting started

This guide takes a new Keel from empty to its first run. You create a workspace, give people access
to it, enrol a host, create a receiver for your Git host, bind a repository, and push. Each step
links to the guide that covers it in full.

The first person to sign in becomes the instance **owner**. This guide assumes you are that person,
or an instance admin.

Keel shows you every change before making it. You fill in a form, choose **Preview** (or the
form's own preview button), check what will change, and choose **Confirm**. A preview lapses after
5 minutes. Some changes also ask for your passkey in a **Verify it's you** dialog. If you have no
passkey yet, Keel asks you to **Register a passkey** first. You can manage your passkeys from the
account menu, under **Passkeys**.

## Create a workspace

A workspace keeps a set of hosts, bindings, inventories and runs apart from everything else. A
workspace holds projects, and each binding belongs to a project.

1. Open **Administration → Teams**.
2. In the **Workspaces** pane, choose **New workspace**.
3. Enter a **Slug**: lowercase letters, digits and dashes, up to 63 characters. For example, `home`.
4. Enter a **Name**, for example `Home lab`.
5. Choose **Preview**, check it, and choose **Confirm**.
6. On the new workspace's row, choose the folder button, **New project in home**. Enter a
   **Project slug** and a **Project name**, choose **Preview**, and confirm.

You will bind a repository to this project later.

## Give people access

People reach a workspace through teams. A team is a group of people, and a grant gives a team one
role in one workspace.

| Role | What it allows |
|---|---|
| Viewer | Seeing the workspace |
| Operator | Also enrolling hosts, syncing bindings, and starting or cancelling runs |
| Approver | Seeing the workspace, and approving or denying reviews |
| Workspace admin | Everything, including bindings, credentials, receivers and trusted signers |

A grant can be limited to some profiles. It then covers only the hosts with one of those
profiles. A grant with no profiles covers the whole workspace.

**Grant yourself access as well.** The Bindings, Receivers, Runs and Reviews pages list only the
workspaces you hold a grant in, even when you are the owner. Until then they say "You have no
workspace yet."

1. In **Administration → Teams**, choose **New team** in the **Teams** pane.
2. Enter a **Team name**, choose **Preview**, and confirm.
3. Select the team to open it.
4. Choose a **User** and select **Add member**. Add yourself first. People appear in the list once
   they have signed in to Keel.
5. Choose the **Workspace** and a **Role**. For yourself, pick **Workspace admin**. Leave the
   profiles empty to cover the whole workspace, and choose **Add grant**.

Adding a grant asks for your passkey.

## Enroll your first host

A host has to enrol before Keel can run anything on it. You get a one-time token in Keel, then run
your base profile on the host with that token.

1. Open **Infrastructure → Hosts** and choose **Enroll host**.
2. Pick the **Workspace**.
3. Enter the **Inventory name**: the host's name in your Ansible inventory, for example `web-01`.
   It must match exactly, because runs find their hosts by this name.
4. Under **Profiles**, keep `base` and add any others this host serves. Press Enter or type a comma
   to add one.
5. Choose **Continue**, check the preview, and choose **Confirm**.
6. Keel shows the **Token** and the **Snippet**. Copy them now: Keel shows them only once, and the
   token works for 15 minutes.
7. On the host, in a checkout of your Ansible repository, run the snippet. The default one asks for
   the token at a hidden prompt, then runs `make keel-node`.
8. Choose **Done**.

When the host connects, it appears in the Hosts table as **online**. If the token expired before
you used it, get a new one from **System Settings → Tokens** with **Roll**.

See [Hosts and enrolment](hosts.md#enroll-a-host) for the details.

## Create a receiver

A receiver is the webhook address your Git host calls on each push. Creating one needs the
Workspace admin role.

1. Open **Pipelines → Receivers** and pick the workspace.
2. Under **New receiver**, enter a **Name** and choose the **Provider**: GitHub, GitLab,
   Forgejo / Gitea, or Generic (HMAC).
3. Choose **Preview receiver**, check it, and confirm.
4. Copy the **Payload URL** and the **Secret**. The secret is shown only once. Then choose
   **I have copied it**.
5. At your Git host, add a webhook for push events with that URL and secret.

See [Receivers](receivers.md#add-the-webhook-at-your-git-host) for each Git host's settings.

## Bind a repository

A binding ties one branch of your Ansible repository to a project. Keel reads the repository's
`.keel/jobs.yaml` to learn its jobs.

1. Open **Pipelines → Bindings** and pick the workspace.
2. If the repository is private, add a credential first. Under **Credentials**, fill in **New
   credential**: a **Name**, the **Kind** (**Git HTTPS token** or **Git SSH key**) and the
   **Value**. Choose **Preview credential** and confirm.
3. Choose **New binding**.
4. Enter the **Project id**, the **Repository URL** and the **Branch**.
5. Pick the **Transport** and the **Credential**. For SSH, also paste the server's **Known host
   line**.
6. Leave **Poll every (seconds)** blank to sync only when a webhook arrives.
7. Choose **Preview binding**. Keel fetches the branch once and shows what it found. Confirm.
8. Back on **Receivers**, find your receiver, choose the binding under **Bind to**, and choose
   **Bind**.

See [Bindings and credentials](bindings.md#bind-a-repository) for the details, including a note on
the project id.

## Your first run

Push a commit to the bound branch. Then:

1. On **Receivers**, the push appears under deliveries as **accepted**. Its **Sync** column shows
   **synced** once Keel has fetched the branch.
2. On **Bindings**, open the binding to see the **Last commit** and the jobs found at it.
3. On **Pipelines → Runs**, a run appears for each job whose `paths` match the files you changed.
   A job without `paths` never starts on a push.

A run reaches the hosts in its inventory that the job's `limit` selects, that are enrolled in the
workspace, and that serve one of the job's profiles.

Each workspace starts with an approval policy that asks for one approval before a run goes ahead,
so a new run first shows as **pending_approval**. The web app has no button to approve such a run
yet.

If the binding is in signed mode and the new commits are not all signed by a trusted signer, the
run waits for a review instead. See [Reviews and trusted signers](reviews.md#approve-or-deny-a-change).

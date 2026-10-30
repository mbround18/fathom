# Receivers

A receiver is how your Git host tells Keel about a push. It is a webhook address with its own
secret: you create it in Keel, add it at your Git host, and every push to a bound branch arrives as
a delivery. This guide covers creating one, adding the webhook, and reading its deliveries.

Keel never trusts the commits listed in a webhook. A delivery only tells Keel that a branch moved;
Keel then fetches the branch itself.

## Create a receiver

Creating, changing and removing receivers needs the Workspace admin role.

1. Open **Pipelines → Receivers** and pick the workspace.
2. Under **New receiver**, enter a **Name**.
3. Choose the **Provider**: **GitHub**, **GitLab**, **Forgejo / Gitea** or **Generic (HMAC)**.
4. Choose **Preview receiver**, check it, and confirm. Keel asks for your passkey.
5. Keel shows the **Payload URL** and the **Secret**. Copy both now: the secret is shown only once.
6. Choose **I have copied it**.

A receiver does nothing until it is bound to a binding. On the receiver's row, choose a binding
under **Bind to** and select **Bind**. Until then, the row says "No binding yet: deliveries are
recorded but sync nothing." One receiver can sync several bindings, and **Unbind** removes one.

### Rotate, disable or remove

- **Rotate secret** makes a new secret and shows it once. The previous secret keeps working for a
  while (24 hours by default), and the row says until when. Update your Git host before then.
- **Disable** stops the receiver from accepting deliveries. Your Git host gets "not found" until you
  enable it again.
- **Remove** deletes the receiver.

## Add the webhook at your Git host

At your Git host, add a webhook on the repository with the **Payload URL** and **Secret** from Keel,
and send push events. Keel records other events as refused.

| Provider | Where to add it | What to set |
|---|---|---|
| GitHub | Repository **Settings → Webhooks → Add webhook** | Payload URL; content type `application/json`; the secret; "Just the push event" |
| GitLab | Project **Settings → Webhooks** | URL; the secret as the **Secret token**; trigger **Push events** |
| Forgejo / Gitea | Repository **Settings → Webhooks → Add webhook** (Forgejo or Gitea) | Target URL; POST content type `application/json`; the secret; trigger on push events |
| Generic (HMAC) | Your own tool | See below |

GitHub sends a ping when you add the webhook. Keel records it and answers it; it starts nothing.

For **Generic (HMAC)**, send a `POST` to the payload URL with a JSON body that has the push's `ref`
and the repository's clone URL. Add these headers:

- `X-Keel-Delivery`: a unique id for each delivery;
- `X-Keel-Signature`: `sha256=` followed by the hex HMAC-SHA256 of the body, keyed with the secret.

Keel treats every generic delivery as a push.

The repository URL in the webhook must match a binding's **Repository URL**. Keel compares them
loosely: `https://`, `ssh://` and `git@host:owner/repo` forms of the same repository match, as do
URLs with and without `.git`, in any case.

## Deliveries

The **Deliveries** table lists each request a receiver got, newest first, and fills in live.

| Column | What it shows |
|---|---|
| **Received** | When it arrived |
| **Receiver** | Which receiver got it |
| **Delivery** | The Git host's delivery id |
| **Decision** | **accepted**, or **refused** with a reason |
| **Sync** | What the fetch did, or "none yet" |
| **Commit** | The commit Keel fetched |

Common reasons for **refused**:

| Reason | What to do |
|---|---|
| `bad_signature` | The secret at your Git host doesn't match. Copy it again, or rotate it. |
| `ref_not_branch` | The push was to a tag or another ref. Only branches sync. |
| `no_matching_binding` | No bound binding has this repository and branch. Check the URL and branch, and that the receiver is bound. |
| `binding_disabled` | The matching binding is disabled. Enable it on **Bindings**. |
| `payload_invalid` | The body wasn't JSON, lacked a ref or URL, or wasn't a push event. |

A delivery the Git host sends twice, with the same delivery id, is recorded once and answered as a
duplicate.

The **Sync** column shows:

| State | Meaning |
|---|---|
| **syncing** | Keel is fetching the branch |
| **synced** | The branch was fetched and the binding moved |
| **duplicate** | The binding was already at that commit, so nothing changed |
| **stale** | The fetched commit is older than the one already synced, so nothing changed |
| **history_rewritten** | The branch was force-pushed. Keel keeps the binding at its last commit and shows the error on the binding |
| **sync_failed** | The fetch failed. Open the binding to see the error |

Keel limits each receiver to 120 deliveries a minute, and each sender to 60. Bodies over 1 MiB are
refused.

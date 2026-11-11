# Reviews and trusted signers

For a binding in signed mode, Keel checks a run's commits before sending it. Commits signed by a
trusted signer go ahead; anything else waits in the review queue until someone approves or denies
it. This guide covers the early check, trusted signers, and reviewing a change.

## The early check

The early check runs only for bindings in signed mode. Bindings are created with signed mode off,
and the web app has no switch for it yet, so on most bindings no run is held for review.

When a run starts on a signed-mode binding, Keel groups its hosts by the commit each one last ran,
and checks every new commit since then against the workspace's trusted signers. If a group's
commits are not all signed by a trusted signer, that group becomes a review, and the run waits in
**needs_review**.

The check is advisory. Each host checks the commits again for itself, against its own trusted
signers, and can refuse a run that Keel let through.

A review gives its reason:

| Why | Meaning |
|---|---|
| **Unsigned commits** | Some new commits are not signed by a trusted signer |
| **Downgrade** | The new commit does not descend from the one the hosts last ran |
| **First signed-mode run (baseline)** | The hosts have no starting commit yet, so only an approved baseline lets them run |
| **Check failed** | Keel could not check the commits, so it holds the run to be safe |

## Trusted signers

Trusted signers are the SSH or OpenPGP keys whose signed commits pass the early check without a
review. They are listed at the bottom of **Pipelines → Reviews**, with each key's **Label**,
**Kind**, **Fingerprint** and when it was **Added**. With none, every commit needs a review.

Each host keeps its own list in `keel_runner_trusted_signers`, and checks commits against that.
Keep the two lists in step: a key only Keel trusts still gets refused by the hosts. See
[Run the base profile](hosts.md#run-the-base-profile).

To add a signer (Workspace admin role):

1. Open **Pipelines → Reviews** in the workspace you want; see [Switch workspace](workspaces.md#switch-workspace).
2. Under **Add a signer**, choose the **Kind**: **SSH** or **OpenPGP**.
3. Paste the **Public key**. For SSH, that is the public key line; for OpenPGP, the armored key.
4. Enter a **Label**, such as the person's name.
5. Choose **Add signer**, check the preview, and confirm with your passkey.

Compare the fingerprint Keel shows with the key's own fingerprint before you rely on it.
**Remove** takes a signer off the list, also with your passkey.

## Approve or deny a change

Approving or denying needs the Approver role, or Workspace admin.

1. Open **Pipelines → Reviews**. **Show** lists **Open** reviews by default.
2. Select a review. It shows the commits with each one's signature status, the files that changed,
   and the diff.
3. Look out for flagged files. They change who or what Keel trusts and targets: **Keel config
   (.keel/)**, **Inventory**, and **keel_runner_\* variables**.
4. Write a **Comment (required)**, up to 2000 characters.
5. Choose **Deny** to stop the change, or **Approve with passkey** to let it run.
6. To approve, Keel shows exactly what your passkey will sign. Choose **Sign and approve** and use
   your passkey. A passkey check from a few minutes ago does not count: approving always asks again.

Your signed approval goes to the hosts, which check it before they run. An approval lapses, at most
24 hours after it is given.

Some reviews need more than one approval. The number is the larger of the binding's quorum and the
hosts' `keel_runner_override_quorum`, and the review's **Approvals** column shows how many it has.
The run goes ahead once every group has enough approvals.

### Four-eyes

When the run's approval policy asks for four-eyes, or the run has no policy, someone other than
whoever asked for the run or wrote these commits must approve. Keel also refuses an approval from
the owner of a trusted signer key that signed one of the commits. If that rules you out, ask
another approver.

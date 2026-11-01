# Runs and approval

A run applies one job from a bound branch to its hosts. Before a run reaches any host, the
workspace's approval policy decides whether someone has to approve it first. This guide covers a
run's states, how a policy holds a run, approving and denying it, four-eyes, the approval window,
and the count of runs waiting for you.

Approval is separate from the review of unsigned commits on a signed-mode binding. That has its own
queue and its own steps: see [Reviews and trusted signers](reviews.md).

## Run states

Each run on **Pipelines → Runs** shows its state.

| State | Meaning |
|---|---|
| **needs review** | Waiting for a review of its commits. See [Reviews](reviews.md) |
| **pending approval** | Waiting for approval. The list shows it as **Waiting**, with its approvals so far, such as **Waiting · 1 of 2** |
| **queued** | Approved, or needing no approval, and about to start |
| **running** | Running on its hosts |
| **succeeded** | Finished on every host without an error |
| **failed** | Could not start, or failed on a host |
| **refused** | A host refused to run it |
| **cancelled** | Someone cancelled it, or its binding was disabled before it started |
| **denied** | An approver denied it. It reads **Approval denied** |
| **expired** | Nobody answered before its approval window closed. It reads **Approval expired** |

A denied, expired or cancelled run never reaches a host.

## How an approval policy holds a run

Every workspace has approval policies. When a run starts, Keel finds the policy that matches its
job, inventory and hosts, and the policy decides:

- whether the run needs approval at all;
- how many approvals it needs;
- whether four-eyes applies: the person who started the run cannot approve it;
- how long the run may wait, its approval window.

Each workspace starts with a catch-all policy that covers every run. It asks for one approval,
without four-eyes, and gives each run 24 hours. The catch-all always asks for approval, so unless a
narrower policy waives it, every new run waits for approval.

A run keeps the terms it was held with. If someone changes or removes the policy while the run
waits, the run still needs the same number of approvals, under the same rules, within the same
window.

## Who can approve

Approving and denying a run needs one of these:

- the **Approver** role in the run's workspace;
- the **Workspace admin** role there;
- being the instance owner or an instance admin.

Anyone who can see the workspace can see its waiting runs. If you cannot approve, the run shows
"An approver or workspace admin can approve this." and no buttons.

## Approving a run

1. Open **Pipelines → Runs** and pick the workspace.
2. Choose **Waiting for approval**. Only waiting runs are listed, oldest first.
3. Open the run. Above its hosts, the approval panel shows:
   - what the policy needs, such as **Needs 1 approval**, and **four-eyes** when it applies;
   - how many approvals the run has, such as **1 of 2 approvals**, with who gave each one and
     when;
   - when the approval window closes.
4. Check what the run will do: the job, the inventory, the hosts it reaches and the commit.
5. Choose **Approve**. The preview says what your approval does: it either queues the run, or adds
   one approval out of the number needed.
6. Choose **Confirm**. If Keel asks you to confirm it's you, use your passkey in the **Verify it's
   you** dialog. If you cancel that, nothing is recorded and the run keeps waiting.

When the run has all the approvals it needs, it is queued and then runs. Until then it keeps
waiting, and another approver has to approve it too. Each person's approval counts once: approving
again, for example from a second browser tab, is refused.

After you approve, the panel says "You approved this run." and shows no buttons.

## Denying a run

Deny a run to stop it. One denial is enough, even when the run already has some of its approvals.

1. Open the waiting run, as in [Approving a run](#approving-a-run).
2. Choose **Deny**.
3. Optionally, write a **Reason**, up to 500 characters. Everyone who opens the run sees it.
4. Choose **Confirm**, and confirm it's you if Keel asks.

The run ends as denied, reads **Approval denied** with your reason, and never reaches a host.

## Four-eyes

When a run's policy uses four-eyes, the person who started the run cannot approve it. Their panel
shows only **Deny**, and the line "Someone else must approve this run." They can still deny it, to
withdraw it.

A run started by a push has no person behind it, so any approver may approve it.

## The approval window

A waiting run has until its approval window closes, counted from when it started waiting. The
window comes from its policy: 24 hours under the catch-all, and at most 30 days under any policy.
The run's panel shows when it closes.

If nobody approves or denies the run in time, Keel ends it as expired within a minute, and it reads
**Approval expired**. That happens even if Keel was stopped when the window closed: it catches up
within a minute of starting again. Approving or denying an expired run is refused with "This run's
approval window has closed."

To run the job after it expired or was denied, start a new run.

## When two people answer at once

If one approver approves while another denies at the same moment, only one answer decides the run.
The other person is told the run has already been answered, and what state it is in now.

## The waiting count

If you can approve in any workspace, the **Runs** link in the navigation shows how many runs are
waiting for an approval you could give. It leaves out the runs you have already approved, and
four-eyes runs you started yourself. The count changes as runs are held, answered or expire,
without reloading the page, and it disappears when nothing is waiting.

## Audit

**Administration → Audit** records every approval, every denial with its reason, every refused
attempt with why it was refused, every expiry, and the moment a run is queued after its last
approval. Each entry names who acted, or Keel for an expiry, and the run, workspace and policy.

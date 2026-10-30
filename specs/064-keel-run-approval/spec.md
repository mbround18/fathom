# Feature Specification: Keel run approval

**Feature Branch**: `064-keel-run-approval`

**Created**: 2026-10-29

**Status**: Draft

**Input**: User description: "Keel run approval (gap found in spec 063 review, 2026-10-29). Spec 055 FR-011/FR-012 require policy-based approval of sensitive runs, and keel-core's run state machine has pending_approval --Approved--> queued and --Denied/Expired--> denied/expired, but keel-server exposes no command to approve or deny a run in pending_approval. Runs held by an approval policy therefore sit until they expire. Add: an Approve and a Deny action for a pending_approval run, gated on the Approve capability (Approver or Workspace admin team roles, plus instance owner/admin) with the existing step-up; the requester may not approve their own run (four-eyes) when the policy says so; audited; respects the policy's approval count and TTL; the Runs page shows pending-approval runs with Approve/Deny for people who can act and who can for those who can't; a pending-approvals count is visible to approvers. Distinct from signed-mode override review, which stays as is. Out of scope: changing policy authoring, signed-mode overrides, host-side verification. Success: a run held by an approval policy can be approved from the UI and runs, denied and stops, or expires, each audited; the guides describe it."

**Completes spec 055**: this implements FR-011 and the approval half of FR-012 for runs that a
policy holds. Signed-mode review (055 FR-034, FR-035) is a different path and does not change.

## Why

Every workspace has a catch-all approval policy that always requires approval and cannot be
weakened. So unless an owner adds a narrower policy that waives approval, every run Keel starts
waits for approval.

Keel can put a run in that waiting state, but nothing can take it out:

- no one can approve it or deny it;
- it does not expire either, because nothing applies the policy's time limit.

The run waits forever, and the guided setup from spec 063 stops at the first push.
`getting-started.md` currently tells the reader that the web app cannot approve such a run.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - An approver lets a held run go ahead (Priority: P1)

A push starts a run, and the policy holds it for approval. An approver opens Runs, sees the run
waiting, and reads what it will do:

- the job;
- the inventory and the hosts it reaches;
- the commit;
- the policy that holds it, and the approvals that policy needs.

The approver confirms who they are and approves. Once the run has the approvals its policy needs,
it is queued and runs.

**Why this priority**: without it, Keel's default policy stops every run. This is the gap that
blocks the first-run walk-through.

**Independent Test**: with a policy that needs one approval, start a run. As an approver, approve
it from Runs. The run moves to queued, then runs. The audit log shows the approval.

**Acceptance Scenarios**:

1. **Given** a run waiting for approval under a policy that needs one approval, **When** an
   approver approves it and confirms their identity, **Then** the run is queued and the audit log
   records who approved it, when, and under which policy.
2. **Given** a policy that needs two approvals, **When** one approver approves, **Then** the run
   keeps waiting and shows "1 of 2 approvals". **When** a second, different approver approves,
   **Then** it is queued.
3. **Given** a four-eyes policy and a run that a person started by hand, **When** that same person
   tries to approve it, **Then** Keel refuses, says why, and records the refusal.
4. **Given** someone who has already approved a run, **When** they try to approve it again,
   **Then** their approval is not counted twice.

---

### User Story 2 - An approver stops a held run (Priority: P1)

An approver looks at a held run and decides it should not go ahead. They deny it, optionally with
a reason. The run ends as denied and never reaches a host.

**Why this priority**: an approval step with only one answer is not a control. Deny ships with
Approve.

**Independent Test**: start a held run, deny it with a reason, and see it end as denied with that
reason. No host receives the job, and the audit log records the denial.

**Acceptance Scenarios**:

1. **Given** a held run, **When** an approver denies it, **Then** it ends as denied ("Approval
   denied"), shows the reason if one was given, and the audit log records the denial.
2. **Given** a run that has one of two approvals, **When** another approver denies it, **Then** it
   ends as denied: a single denial is enough.
3. **Given** a four-eyes policy, **When** the person who started the run denies it, **Then** Keel
   accepts the denial: four-eyes guards approving, not withdrawing.

---

### User Story 3 - Held runs don't wait forever (Priority: P2)

A held run that nobody answers expires once its policy's approval window has passed. It ends as
expired ("Approval expired"), so the queue holds only runs someone can still act on.

**Why this priority**: today held runs pile up forever. Expiry keeps the queue honest, but
approve and deny matter more.

**Independent Test**: with a policy whose approval window is short, start a held run and wait past
the window. The run ends as expired, the audit log records it, and approving it afterwards is
refused.

**Acceptance Scenarios**:

1. **Given** a held run whose approval window has passed, **When** Keel next checks, **Then** the
   run ends as expired and the audit log records it.
2. **Given** an approver with an expired run open on screen, **When** they approve it, **Then** Keel
   refuses ("This run's approval window has closed") and the page shows the run as expired.
3. **Given** a Keel server that was down when a window closed, **When** it starts again, **Then**
   the run expires on start-up rather than staying held.

---

### User Story 4 - Approvers can see what is waiting for them (Priority: P2)

An approver sees how many runs are waiting for their approval, in every workspace where they can
approve, without opening each one. People who cannot approve see that a run is waiting, and who
can approve it, with no buttons they cannot use.

**Why this priority**: an approval that nobody notices is the same as no approval. It depends on
stories 1 and 2.

**Independent Test**: as an approver, start two held runs and see a count of 2 next to Runs, and a
filter that lists only those two. As a viewer, open the same runs and see "An approver can approve
this" and no Approve or Deny.

**Acceptance Scenarios**:

1. **Given** held runs in a workspace, **When** an approver for that workspace looks at the
   navigation, **Then** Runs shows how many are waiting. **When** the last one is answered, the
   count disappears.
2. **Given** a person who cannot approve in the workspace, **When** they open a held run, **Then**
   they see that it is waiting, who can approve it, and no Approve or Deny buttons.
3. **Given** the Runs page, **When** an approver chooses "Waiting for approval", **Then** only held
   runs are listed, oldest first.

---

### Edge Cases

- **Two approvers answer at once.** One approves and the other denies at the same moment. Exactly
  one answer decides the run. The other is told the run has already been answered, and both
  attempts are audited.
- **The binding is disabled** while a run is held. The run is cancelled, as it already is today
  ("Binding disabled"), and Approve and Deny are refused.
- **The policy changes** after the run was held. The run keeps the approval count, four-eyes rule
  and window it was held with. A later edit does not loosen or tighten a run already waiting.
- **The policy is removed** after the run was held. The same rule applies: the run keeps what it
  was held with.
- **An approver loses their role** after approving, but before the run has enough approvals. Their
  approval still counts: it was valid when given. The audit log shows their role at that time.
- **A run with no person behind it** (started by a push). Four-eyes has no requester to exclude,
  so any approver may approve it.
- **The same approver acts twice**, for example from two browser tabs. The second approval is
  refused as a duplicate and is not counted.
- **Identity confirmation is cancelled or fails.** Nothing is recorded as an approval, and the run
  stays held.
- **A run held for signed-mode review** (`needs_review`) is not affected by this feature. It keeps
  its review flow.

## Requirements *(mandatory)*

### Functional Requirements

**Approving and denying**

- **FR-001**: A person with the Approve capability in the run's workspace MUST be able to approve
  a run that is waiting for approval. That means an Approver or Workspace admin grant, or an
  instance owner or admin.
- **FR-002**: The same people MUST be able to deny a waiting run, with an optional reason of at
  most 500 characters. One denial ends the run as denied.
- **FR-003**: Approving and denying MUST ask the person to confirm their identity with the
  existing step-up check (spec 054) before Keel records the decision. A cancelled or failed check
  records nothing.
- **FR-004**: A run MUST be queued only once it has as many distinct approvals as the policy it
  was held under requires. Until then it shows how many it has and how many it needs.
- **FR-005**: Under a four-eyes policy, Keel MUST refuse an approval from the person who started
  the run, and say why. A run that no person started has no one to exclude.
- **FR-006**: Keel MUST refuse a second approval of the same run by the same person, and MUST NOT
  count it.
- **FR-007**: Keel MUST refuse Approve and Deny on a run that is no longer waiting for approval
  (queued, running, ended, cancelled or expired), and say what state it is in.
- **FR-008**: When answers race, exactly one MUST decide the run. Every attempt, including the
  ones refused, MUST be audited.
- **FR-009**: A run MUST keep the approval count, four-eyes rule and approval window of the policy
  it was held under, even if that policy later changes or is removed.

**Expiry**

- **FR-010**: A held run MUST expire once its approval window has passed. The window is counted
  from when the run started waiting, and taken from the policy it was held under. The run ends as
  expired ("Approval expired").
- **FR-011**: Expiry MUST also happen for runs whose window closed while Keel was not running.
  Keel catches up within one minute of starting.
- **FR-012**: Keel MUST apply expiry within one minute of the window closing.

**Audit**

- **FR-013**: Keel MUST write an audit entry for each of these:
  - each approval;
  - each denial;
  - each refused attempt, with its reason;
  - each expiry;
  - the run being queued after its final approval.
- **FR-014**: Each audit entry MUST name the actor (or "Keel" for an expiry), the run, the
  workspace, the policy and the outcome. A denial's entry also has its reason.

**Showing it**

- **FR-015**: A held run MUST show:
  - its job, inventory, hosts and commit;
  - the policy that holds it;
  - its approvals so far, with who and when;
  - the approvals it needs;
  - when its window closes.
- **FR-016**: Someone who can approve MUST see Approve and Deny on a held run.
  - Someone who cannot MUST see who can approve ("An approver or workspace admin can approve
    this"), and no buttons.
  - The person who started a four-eyes run sees Deny, and a line saying someone else must
    approve it.
- **FR-017**: Runs MUST offer a "Waiting for approval" filter, listing held runs oldest first.
- **FR-018**: For people who can approve somewhere, the navigation MUST show next to Runs how many
  runs are waiting for an approval they could give. The count updates as runs are held, answered
  or expire, without a page reload.
- **FR-019**: The Approve and Deny buttons MUST follow Keel's existing confirmation pattern: a
  preview of what the decision does, then confirm.
  - Approving uses the primary (accent) style.
  - Denying uses the destructive style.

**Guides**

- **FR-020**: `docs/guides/keel/getting-started.md` MUST replace its note that runs cannot be
  approved in the web app with the real steps.
- **FR-021**: A runs guide (`docs/guides/keel/runs.md`) MUST describe:
  - approval policies as they affect a run;
  - approving, denying and expiry;
  - four-eyes.
- **FR-022**: The guided Runs empty state and its glossary entries (spec 063) MUST link to the
  runs guide.

**Boundaries**

- **FR-023**: Signed-mode review (`needs_review` runs, overrides, baselines and trusted signers)
  MUST NOT change.
- **FR-024**: Policy authoring MUST NOT change: the fields, their limits and the catch-all rule
  stay as they are.
- **FR-025**: Hosts MUST NOT need any change. A policy approval is a decision inside Keel, and the
  run reaches hosts the way any queued run does.

### Key Entities

- **Run approval**: one person's approval of one run. It records:
  - the run;
  - who approved;
  - their role at the time;
  - when.

  It is append-only. A run has zero or more approvals, and at most one per person.
- **Run denial**: the decision that ended a held run. It records who denied it, when and the
  optional reason. A run has at most one.
- **Held run's terms**: the approval count, four-eyes rule and window copied from the policy when
  the run started waiting (FR-009).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Under the default catch-all policy, an owner can go from a push to a running job
  without leaving Keel. The approval itself takes under one minute.
- **SC-002**: No held run stays waiting more than one minute past its approval window, including
  across a Keel restart.
- **SC-003**: Each of these produces exactly one audit entry naming the actor and the outcome:
  - approve;
  - deny;
  - expire;
  - refused attempts, which are audited too.
- **SC-004**: When two people answer the same run at the same moment, the run ends in one decided
  state every time.
- **SC-005**: A person who cannot approve never sees an Approve button. A person who can approve
  sees the waiting count within a few seconds of a run being held.
- **SC-006**: The first-run walk-through from spec 063 (SC-001 there) can be completed end to end.

## Assumptions

- **Who started a run**: Keel already records it for runs started by hand, and records no one for
  runs started by a push. Four-eyes excludes only a recorded starter. Matching commit authors to
  Keel users is out of scope.
- **Step-up**: spec 054's step-up check is the identity confirmation. Unlike a signed-mode
  override, approving does not need a passkey signature over the run, because hosts don't check
  policy approvals (FR-025).
- **Approval window**:
  - The policy's existing approval window applies: default 24 hours, at most 30 days.
  - A run held with no policy, because its policy was removed before the run, uses the catch-all's
    window.
- **Changes made by others**: the waiting count and the run list update with Keel's existing live
  updates. No new notification channel, such as email or chat, is added.
- **Viewing held runs**: a person who can see a workspace can see its held runs, as they can see
  any run today.
- **Out of scope**:
  - changing policies;
  - signed-mode overrides;
  - host-side verification;
  - notifications outside Keel;
  - delegating approvals;
  - approving several runs at once.

-- Spec 064: approving and denying runs held by an approval policy. Additive only.
--
-- A run copies its policy's terms when it starts waiting for approval, so a later policy change
-- never moves the goalposts for a run already waiting (FR-009).
alter table runs
    add column approvals_needed integer check (approvals_needed >= 1),
    add column four_eyes boolean,
    add column approval_expires_at timestamptz;

-- Runs already waiting take their policy's terms, or the catch-all defaults when the policy is
-- gone, counted from when the run was made.
update runs r
   set approvals_needed = coalesce(p.approvals_needed, 1),
       four_eyes = coalesce(p.four_eyes, false),
       approval_expires_at = r.created_at + coalesce(p.approval_ttl, interval '24 hours')
  from runs r2
  left join approval_policies p on p.id = r2.policy_id
 where r2.id = r.id
   and r.state = 'pending_approval';

alter table runs add constraint runs_held_terms check (
    state <> 'pending_approval'
    or (approvals_needed is not null and four_eyes is not null and approval_expires_at is not null)
);

-- The sweeper and the pending-approval count.
create index runs_pending_approval on runs (approval_expires_at) where state = 'pending_approval';

-- The approver's effective workspace role when they answered.
alter table approvals add column approver_role text
    check (approver_role in ('approver', 'ws_admin', 'owner', 'admin'));

-- Answers are a record: insert-only for the serving role (already so since 0007; restated).
revoke update, delete, truncate on approvals from keel_app;

-- Spec 068: Keel converge. A converge is one parent run (`kind = 'converge'`) whose steps are
-- child runs, one per distinct job; each host walks its steps in order under a per-host hold
-- (`converge_hosts_one_holding`), advanced by the `converge_advance` trigger on `run_hosts`.
-- Also: commit-status gates, profile suggestions and auto-converge on enrollment.
--
-- Numbering: 0015, 0017 and 0018 (`join`) are spec 067's; if another branch lands 0019 first,
-- renumber this file before any shared database applies it (data-model.md).

-- ---- runs ----------------------------------------------------------------------------------
-- The parent of a converge has no job: `job_id` is null and `job_name` is the literal
-- `converge` (the column stays not null so every reader keeps working).
alter table runs drop constraint runs_kind_check;
alter table runs add constraint runs_kind_check
    check (kind in ('normal', 'rollback', 'baseline', 'converge'));
alter table runs drop constraint runs_state_check;
alter table runs add constraint runs_state_check check (state in (
    'needs_review', 'pending_approval', 'waiting_checks', 'queued', 'running', 'succeeded',
    'failed', 'refused', 'cancelled', 'denied', 'expired'
));
alter table runs drop constraint runs_state_reason_check;
alter table runs add constraint runs_state_reason_check check (state_reason in (
    'approval_denied', 'approval_expired', 'binding_disabled', 'cancelled_by_user',
    'runner_lost', 'inventory_invalid', 'artifact_missing', 'credential_unavailable',
    'host_refused', 'checks_failed', 'checks_timeout', 'replaced', 'runner_too_old'
));
alter table runs
    add column parent_run_id uuid references runs (id) on delete cascade,
    add column check_mode boolean not null default false,
    add column retry_of uuid references runs (id) on delete set null,
    -- FR-020: the vars files a path-triggered run was narrowed by; null when not narrowed.
    add column narrowed_by text[];
alter table runs add constraint runs_converge_shape check (
    (kind <> 'converge' or (job_id is null and parent_run_id is null))
    and (parent_run_id is null or kind = 'normal')
);
create index runs_parent on runs (parent_run_id) where parent_run_id is not null;

-- ---- run_hosts -----------------------------------------------------------------------------
-- `blocked`: a converge step waiting on approval or on the host's previous step.
alter table run_hosts drop constraint run_hosts_state_check;
alter table run_hosts add constraint run_hosts_state_check check (state in (
    'blocked', 'queued', 'sent', 'running', 'succeeded', 'failed', 'refused', 'lost',
    'cancelled', 'skipped', 'covered'
));
alter table run_hosts drop constraint run_hosts_refusal_check;
alter table run_hosts add constraint run_hosts_refusal_check check (refusal in (
    'unsigned', 'bad_override', 'base_mismatch', 'profile_not_allowed', 'downgrade',
    'replay', 'ledger_corrupt', 'version_too_old', 'params_mismatch', 'runner_too_old',
    'choice_mismatch', 'env_refused'
));
alter table run_hosts
    add column covered_by uuid references runs (id) on delete set null,
    add column skip_reason text
        check (skip_reason in ('dependency_failed', 'converge_cancelled'));

-- ---- bindings and enrollments --------------------------------------------------------------
alter table repo_bindings
    add column checks_timeout_secs integer not null default 3600
        check (checks_timeout_secs between 60 and 86400),
    -- Set by sync when the manifest uses a key only a 068 runner reads (contracts/protocol.md).
    add column manifest_v068 boolean not null default false;
alter table host_enrollments add column auto_converge boolean not null default true;

-- ---- credentials -------------------------------------------------------------------------
-- `env_value`: one value set as the environment variable a job's `env:` names.
alter table credentials drop constraint credentials_kind_check;
alter table credentials add constraint credentials_kind_check check (
    kind in ('vault_password', 'env', 'secret_vars', 'git_https_token', 'git_ssh_key',
             'forge_token', 'env_value')
);

-- ---- converge ------------------------------------------------------------------------------
create table converge_hosts (
    run_id uuid not null references runs (id) on delete cascade,
    host_id uuid not null references hosts (id),
    state text not null default 'waiting'
        check (state in ('waiting', 'holding', 'succeeded', 'failed', 'cancelled')),
    profiles text[] not null,
    started_at timestamptz,
    ended_at timestamptz,
    primary key (run_id, host_id)
);
-- The per-host lock: one converge holds a host at a time.
create unique index converge_hosts_one_holding on converge_hosts (host_id) where state = 'holding';
create index converge_hosts_open on converge_hosts (host_id) where state in ('waiting', 'holding');

create table converge_steps (
    run_id uuid not null references runs (id) on delete cascade,
    host_id uuid not null references hosts (id),
    position smallint not null check (position between 0 and 31),
    job_name text not null,
    step_run_id uuid not null references runs (id) on delete cascade,
    via text[] not null default '{}',
    primary key (run_id, host_id, position),
    unique (run_id, host_id, job_name)
);
create index converge_steps_step on converge_steps (step_run_id, host_id);

-- ---- suggestions, required checks ----------------------------------------------------------
create table profile_suggestions (
    host_id uuid not null references hosts (id) on delete cascade,
    profile text not null,
    action text not null check (action in ('add', 'remove')),
    groups text[] not null,
    binding_id uuid not null references repo_bindings (id) on delete cascade,
    commit text not null,
    first_seen_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now(),
    dismissed_at timestamptz,
    dismissed_by uuid references users (id) on delete set null,
    primary key (host_id, profile, action)
);

create table binding_required_checks (
    binding_id uuid not null references repo_bindings (id) on delete cascade,
    context text not null check (char_length(context) between 1 and 200),
    created_at timestamptz not null default now(),
    created_by uuid references users (id) on delete set null,
    primary key (binding_id, context)
);

create table run_check_states (
    run_id uuid not null references runs (id) on delete cascade,
    context text not null,
    state text not null check (state in ('pending', 'success', 'failure', 'error', 'missing')),
    target_url text,
    updated_at timestamptz not null default now(),
    primary key (run_id, context)
);

grant select, insert, update on converge_hosts, converge_steps, run_check_states to keel_app;
-- Rows a sync no longer produces, and checks a binding no longer requires, are deleted.
grant select, insert, update, delete on profile_suggestions, binding_required_checks to keel_app;

-- ---- converge_advance ----------------------------------------------------------------------
-- When a step's host row ends, the host's next step moves: succeeded ⇒ the next one is queued
-- (or the host's converge succeeds); anything else ⇒ the later steps are skipped and the host's
-- converge fails (or is cancelled). The `skipped` updates re-enter this trigger with nothing
-- later left to skip, so the recursion ends. In-database so a crash between "step finished" and
-- "next step queued" can't happen (research R2).
create function converge_advance() returns trigger language plpgsql as $$
declare
    step converge_steps%rowtype;
    nxt converge_steps%rowtype;
begin
    select * into step from converge_steps
     where step_run_id = new.run_id and host_id = new.host_id;
    if not found then
        return null;
    end if;
    if new.state = 'succeeded' then
        select * into nxt from converge_steps
         where run_id = step.run_id and host_id = step.host_id and position > step.position
         order by position limit 1;
        if found then
            update run_hosts set state = 'queued'
             where run_id = nxt.step_run_id and host_id = nxt.host_id and state = 'blocked';
        else
            update converge_hosts set state = 'succeeded', ended_at = now()
             where run_id = step.run_id and host_id = step.host_id
               and state in ('waiting', 'holding');
        end if;
    else
        -- The host's converge ends first, so a re-entering `skipped` finds it already over.
        update converge_hosts
           set state = case when new.state = 'cancelled' then 'cancelled' else 'failed' end,
               ended_at = now()
         where run_id = step.run_id and host_id = step.host_id
           and state in ('waiting', 'holding');
        update run_hosts rh
           set state = 'skipped', ended_at = now(),
               skip_reason = case when new.state = 'cancelled' then 'converge_cancelled'
                                  else 'dependency_failed' end
          from converge_steps s
         where s.run_id = step.run_id and s.host_id = step.host_id
           and s.position > step.position
           and rh.run_id = s.step_run_id and rh.host_id = s.host_id and rh.state = 'blocked';
    end if;
    return null;
end;
$$;

create trigger converge_advance
    after update of state on run_hosts
    for each row
    when (old.state in ('blocked', 'queued', 'sent', 'running')
          and new.state in ('succeeded', 'failed', 'refused', 'lost', 'cancelled', 'skipped',
                            'covered'))
    execute function converge_advance();

-- Approval policies, runs and their per-host state, results, events and logs (spec 055 US2,
-- T065; data-model.md "Jobs, policies and runs"). `hosts` and `host_profiles` are the minimum
-- runs need to resolve and dispatch to enrolled hosts; enrollment (US7) extends them.

create table approval_policies (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    priority integer not null,
    match jsonb not null default '{}',
    requires_approval boolean not null,
    approvals_needed integer not null default 1 check (approvals_needed >= 1),
    four_eyes boolean not null default false,
    approval_ttl interval not null default interval '24 hours'
        check (approval_ttl > interval '0' and approval_ttl <= interval '30 days'),
    created_at timestamptz not null default now(),
    unique (workspace_id, priority)
);

-- The catch-all (priority max) cannot be removed while its workspace exists, cannot stop
-- requiring approval, and cannot narrow what it matches (R6).
create function approval_policies_guard() returns trigger
language plpgsql as $$
begin
    if tg_op = 'DELETE' then
        if old.priority = 2147483647
            and exists (select 1 from workspaces where id = old.workspace_id) then
            raise exception 'the catch-all approval policy cannot be removed'
                using errcode = 'check_violation';
        end if;
        return old;
    end if;
    if old.priority = 2147483647 and (
        new.priority <> old.priority or not new.requires_approval or new.match <> '{}'::jsonb
    ) then
        raise exception 'the catch-all approval policy cannot be weakened'
            using errcode = 'check_violation';
    end if;
    return new;
end
$$;
create trigger approval_policies_guard before update or delete on approval_policies
    for each row execute function approval_policies_guard();

create function workspaces_catch_all() returns trigger
language plpgsql as $$
begin
    insert into approval_policies (id, workspace_id, priority, match, requires_approval)
        values (gen_random_uuid(), new.id, 2147483647, '{}', true);
    return new;
end
$$;
create trigger workspaces_catch_all after insert on workspaces
    for each row execute function workspaces_catch_all();

insert into approval_policies (id, workspace_id, priority, match, requires_approval)
    select gen_random_uuid(), w.id, 2147483647, '{}', true from workspaces w
    where not exists (
        select 1 from approval_policies p where p.workspace_id = w.id and p.priority = 2147483647
    );

-- One per enrolled host runner (R1). Enrollment (US7) fills the key and versions.
create table hosts (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    inventory_name text not null check (char_length(inventory_name) between 1 and 253),
    ssh_key_fingerprint text unique,
    ssh_public_key text,
    allowed_profiles text[] not null default '{}',
    require_signed boolean not null default false,
    runner_version text,
    ansible_version text,
    state text not null default 'offline' check (state in ('online', 'offline', 'revoked')),
    enrolled_at timestamptz not null default now(),
    last_seen_at timestamptz,
    revoked_at timestamptz,
    unique (workspace_id, inventory_name)
);

create table host_profiles (
    host_id uuid not null references hosts (id) on delete cascade,
    profile_id uuid not null references profiles (id) on delete cascade,
    assigned_by uuid references users (id) on delete set null,
    assigned_at timestamptz not null default now(),
    primary key (host_id, profile_id)
);

create table runs (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    -- Jobs and bindings can be removed; the run's history stays, named.
    job_id uuid references jobs (id) on delete set null,
    binding_id uuid references repo_bindings (id) on delete set null,
    job_name text not null,
    kind text not null default 'normal' check (kind in ('normal', 'rollback', 'baseline')),
    reverses_run_id uuid references runs (id),
    trigger text not null check (trigger in ('delivery', 'poll', 'manual', 'api', 'workflow')),
    delivery_id uuid references deliveries (id) on delete set null,
    requested_by uuid references users (id) on delete set null,
    commit text not null,
    -- Null only when no revision could be pinned (the run fails `inventory_invalid`).
    inventory_revision_id uuid references inventory_revisions (id),
    inventory_name text not null,
    resolved_hosts text[] not null default '{}',
    run_limit text,
    policy_id uuid references approval_policies (id) on delete set null,
    signed_mode boolean not null default false,
    state text not null check (state in (
        'needs_review', 'pending_approval', 'queued', 'running', 'succeeded', 'failed',
        'refused', 'cancelled', 'denied', 'expired'
    )),
    state_reason text check (state_reason in (
        'approval_denied', 'approval_expired', 'binding_disabled', 'cancelled_by_user',
        'runner_lost', 'inventory_invalid', 'artifact_missing', 'credential_unavailable',
        'host_refused'
    )),
    created_at timestamptz not null default now(),
    queued_at timestamptz,
    started_at timestamptz,
    ended_at timestamptz,
    summary jsonb
);
create index runs_workspace_created on runs (workspace_id, created_at desc);
create index runs_binding on runs (binding_id, created_at desc);
create index runs_active on runs (state) where state in ('queued', 'running');

create table run_hosts (
    run_id uuid not null references runs (id) on delete cascade,
    host_id uuid not null references hosts (id),
    host_name text not null,
    base_commit text,
    state text not null default 'queued' check (state in (
        'queued', 'sent', 'running', 'succeeded', 'failed', 'refused', 'lost', 'cancelled'
    )),
    refusal text check (refusal in (
        'unsigned', 'bad_override', 'base_mismatch', 'profile_not_allowed', 'downgrade',
        'replay', 'ledger_corrupt', 'version_too_old'
    )),
    ledger_hash text,
    started_at timestamptz,
    ended_at timestamptz,
    primary key (run_id, host_id)
);
create index run_hosts_queued on run_hosts (host_id) where state = 'queued';

-- Immutable: written once when a host finishes.
create table run_host_results (
    run_id uuid not null references runs (id) on delete cascade,
    host_id uuid not null references hosts (id),
    ok integer not null default 0,
    changed integer not null default 0,
    failed integer not null default 0,
    unreachable integer not null default 0,
    skipped integer not null default 0,
    rescued integer not null default 0,
    ignored integer not null default 0,
    first_failure jsonb,
    primary key (run_id, host_id)
);

-- Append-only, batched from the runner's `keel_events` stream (R3). `seq` is run-wide and
-- shared with `run_log_chunks`, so a subscriber resumes both from one number.
create table run_events (
    run_id uuid not null references runs (id) on delete cascade,
    seq bigint not null check (seq >= 1),
    at timestamptz not null default now(),
    kind text not null check (char_length(kind) <= 64),
    host text,
    task text,
    data jsonb not null default '{}',
    primary key (run_id, seq)
);

-- Append-only, bounded per run by `--run-log-cap`: past it, the head and tail are kept with a
-- truncation marker between them.
create table run_log_chunks (
    run_id uuid not null references runs (id) on delete cascade,
    seq bigint not null check (seq >= 1),
    stream text not null check (stream in ('stdout', 'stderr')),
    bytes bytea not null,
    primary key (run_id, seq)
);

grant select, insert, update, delete on approval_policies, hosts, host_profiles, runs, run_hosts
    to keel_app;
grant select, insert on run_host_results, run_events, run_log_chunks to keel_app;
revoke update, delete, truncate on run_host_results, run_events, run_log_chunks from keel_app;

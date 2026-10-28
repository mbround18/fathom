-- Runs outside Keel (spec 058 amendment A2, T041): what each host's runner reports from its
-- `/var/lib/keel-outside/runs.jsonl`. These rows are the host's claims, kept as received.
-- Insert-only, like `audit_event`: a trigger refuses UPDATE, DELETE and TRUNCATE for everyone,
-- and `keel_app` holds SELECT and INSERT only. Additive only.
create table host_outside_runs (
    id uuid primary key default gen_random_uuid(),
    host_id uuid not null references hosts (id),
    at timestamptz not null,
    received_at timestamptz not null default now(),
    "user" text not null check (octet_length("user") <= 64),
    playbook text not null check (octet_length(playbook) <= 256),
    commit text check (commit is null or commit ~ '^[0-9a-f]{40}$'),
    dirty boolean not null,
    "check" boolean not null
);
-- A runner that re-reads its record (truncated, replaced, or an unacknowledged batch) sends
-- entries again; they are stored once.
create unique index host_outside_runs_dedup
    on host_outside_runs (host_id, at, playbook, coalesce(commit, ''));
create index host_outside_runs_host_at on host_outside_runs (host_id, at desc);

create function host_outside_runs_refuse_change() returns trigger
language plpgsql as $$
begin
    raise exception 'host_outside_runs is insert-only (% refused)', tg_op
        using errcode = 'insufficient_privilege';
end;
$$;

create trigger host_outside_runs_no_update_delete
    before update or delete on host_outside_runs
    for each row execute function host_outside_runs_refuse_change();

create trigger host_outside_runs_no_truncate
    before truncate on host_outside_runs
    for each statement execute function host_outside_runs_refuse_change();

revoke all on host_outside_runs from public;
grant select, insert on host_outside_runs to keel_app;
revoke update, delete, truncate on host_outside_runs from keel_app;

-- Set by an entry without `--check`; cleared by a baseline approval for the host, or by a
-- completed run of every job assigned to it after the newest such entry.
alter table hosts add column changed_outside boolean not null default false;

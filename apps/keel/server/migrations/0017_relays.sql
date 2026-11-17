-- Spec 067: relays, zones and host queries (data-model.md). Additive only.
--
-- 0015 and 0016 are reserved for other branches; sqlx applies unapplied versions in order, so
-- they land later without trouble.

-- A zone groups hosts behind the same relays. No zone (null) is the implicit `direct` zone,
-- which is why the name is reserved.
create table zones (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    name text not null check (name ~ '^[a-z0-9][a-z0-9-]{0,62}$' and name <> 'direct'),
    description text not null default '' check (char_length(description) <= 500),
    created_at timestamptz not null default now(),
    unique (workspace_id, name),
    -- Target of the same-workspace foreign keys below.
    unique (id, workspace_id)
);

-- A host's zone must be in its workspace; null keeps it direct. A zone with hosts cannot be
-- removed (no cascade).
alter table hosts
    add column zone_id uuid,
    add column last_path jsonb check (last_path is null or jsonb_typeof(last_path) = 'object'),
    add constraint hosts_zone_same_workspace foreign key (zone_id, workspace_id)
        references zones (id, workspace_id);
create index hosts_zone on hosts (zone_id) where zone_id is not null;

-- Keel's own key for dialling relays, shaped like `ssh_cas`. The private key is sealed with the
-- master key under AAD `relay_link_key:<id>`. Retired, never deleted.
create table relay_link_keys (
    id uuid primary key,
    public_key text not null,
    fingerprint text not null unique,
    sealed_private_key bytea not null,
    state text not null check (state in ('pending', 'active', 'retired')),
    created_at timestamptz not null default now(),
    activated_at timestamptz,
    retired_at timestamptz
);
create unique index relay_link_keys_one_active on relay_link_keys (state) where state = 'active';

-- A relay Keel dials. Live state is in memory (the relay supervisor); only what an owner set is
-- stored, plus the quarantine Keel sets on a protocol violation, which only an owner release
-- clears. Revoked, never deleted.
create table relays (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    zone_id uuid not null,
    name text not null check (name ~ '^[a-z0-9][a-z0-9-]{0,62}$' and name <> 'direct'),
    dial_address text not null check (char_length(dial_address) between 3 and 300),
    host_key text not null check (host_key like 'ssh-ed25519 %'),
    host_key_fingerprint text not null unique,
    host_id uuid references hosts (id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    revoked_at timestamptz,
    quarantined_at timestamptz,
    quarantine_reason text,
    unique (workspace_id, name),
    foreign key (zone_id, workspace_id) references zones (id, workspace_id),
    constraint relays_quarantine_has_reason
        check ((quarantined_at is null) = (quarantine_reason is null))
);
create index relays_zone on relays (zone_id) where revoked_at is null;

-- Read-only questions put to a host's runner. The newest 200 rows per host are kept; older ones
-- are deleted in the same transaction as each insert, so keel_app may delete here (only here).
create table host_queries (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    host_id uuid not null references hosts (id) on delete cascade,
    kind text not null check (
        kind in ('facts', 'applied_revision', 'unit_state', 'disk_health', 'path_free')
    ),
    params jsonb not null default '{}' check (jsonb_typeof(params) = 'object'),
    asked_by uuid not null references users (id),
    asked_at timestamptz not null default now(),
    outcome text check (
        outcome in ('answered', 'refused', 'timeout', 'no_capability', 'disconnected')
    ),
    answer jsonb check (answer is null or octet_length(answer::text) <= 65536),
    refusal text,
    answered_at timestamptz
);
create index host_queries_host on host_queries (host_id, asked_at desc);

grant select, insert, update on zones, relay_link_keys, relays, host_queries to keel_app;
grant delete on host_queries to keel_app;
revoke delete, truncate on zones, relay_link_keys, relays from keel_app;
revoke truncate on host_queries from keel_app;

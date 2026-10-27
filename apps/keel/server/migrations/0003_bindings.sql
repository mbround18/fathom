-- Repository bindings, receivers, deliveries, credentials, jobs and profiles (spec 055 US1,
-- T044; data-model.md "Repositories and receivers", "Credentials", "Jobs").

-- Sealed at rest (seal.rs, AAD `credential:<id>`). Values are never returned.
create table credentials (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    name text not null check (name ~ '^[a-z0-9][a-z0-9_-]{0,62}$'),
    kind text not null check (
        kind in ('vault_password', 'env', 'secret_vars', 'git_https_token', 'git_ssh_key')
    ),
    sealed bytea not null,
    fingerprint text not null,
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now(),
    rotated_at timestamptz,
    unique (workspace_id, name)
);

create table repo_bindings (
    id uuid primary key,
    project_id uuid not null references projects (id) on delete cascade,
    url text not null check (char_length(url) between 1 and 2048),
    -- repo_url::normalize(url): what receivers match payload URLs against.
    url_key text not null,
    branch text not null check (char_length(branch) between 1 and 255),
    transport text not null check (transport in ('https', 'ssh')),
    credential_id uuid references credentials (id) on delete restrict,
    known_host text,
    allow_force_push boolean not null default false,
    poll_interval integer check (poll_interval is null or poll_interval between 60 and 86400),
    signed_mode boolean not null default false,
    override_quorum integer not null default 1 check (override_quorum between 1 and 16),
    status text not null default 'active' check (status in ('active', 'disabled')),
    last_commit text,
    last_synced_at timestamptz,
    last_sync_error text,
    manifest_problems jsonb not null default '[]',
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now(),
    unique (url, branch, project_id),
    check (transport <> 'ssh' or known_host is not null)
);
create index repo_bindings_url_key on repo_bindings (url_key, branch);

create table credential_grants (
    credential_id uuid not null references credentials (id) on delete cascade,
    binding_id uuid not null references repo_bindings (id) on delete cascade,
    profiles text[] not null default '{}',
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now(),
    primary key (credential_id, binding_id)
);

-- Secrets sealed with AAD `receiver:<id>`.
create table receivers (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    name text not null check (name ~ '^[a-z0-9][a-z0-9_-]{0,62}$'),
    provider text not null check (provider in ('github', 'gitlab', 'forgejo', 'generic')),
    secret_current bytea not null,
    secret_previous bytea,
    previous_expires_at timestamptz,
    status text not null default 'active' check (status in ('active', 'disabled')),
    created_at timestamptz not null default now(),
    rotated_at timestamptz,
    unique (workspace_id, name)
);

create table receiver_bindings (
    receiver_id uuid not null references receivers (id) on delete cascade,
    binding_id uuid not null references repo_bindings (id) on delete cascade,
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now(),
    primary key (receiver_id, binding_id)
);

create table deliveries (
    id uuid primary key,
    receiver_id uuid not null references receivers (id) on delete cascade,
    provider_delivery_id text not null check (char_length(provider_delivery_id) <= 200),
    received_at timestamptz not null default now(),
    binding_id uuid references repo_bindings (id) on delete set null,
    decision text not null check (decision in ('accepted', 'refused')),
    refusal text check (refusal in (
        'bad_signature', 'unknown_receiver', 'receiver_disabled', 'no_matching_binding',
        'binding_disabled', 'ref_not_branch', 'payload_invalid'
    )),
    sync_state text check (sync_state in (
        'syncing', 'synced', 'duplicate', 'stale', 'history_rewritten', 'sync_failed'
    )),
    commit text,
    check ((decision = 'refused') = (refusal is not null))
);
-- A delivery id counts once per receiver, but only once its signature checked out: an unsigned
-- request must not be able to claim an id ahead of the real delivery.
create unique index deliveries_once on deliveries (receiver_id, provider_delivery_id)
    where refusal is distinct from 'bad_signature' and refusal is distinct from 'receiver_disabled';
create index deliveries_received on deliveries (receiver_id, received_at desc);

-- Synced from `.keel/jobs.yaml` at the binding's fetched head.
create table jobs (
    id uuid primary key,
    binding_id uuid not null references repo_bindings (id) on delete cascade,
    name text not null,
    definition jsonb not null,
    blocked_reason text,
    defined_at_commit text not null,
    removed_at_commit text,
    unique (binding_id, name)
);

create table profiles (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    name text not null check (name ~ '^[a-z0-9][a-z0-9_-]{0,62}$'),
    description text,
    declared_at_commit text,
    unique (workspace_id, name)
);

grant select, insert, update, delete on
    credentials, repo_bindings, credential_grants, receivers, receiver_bindings, deliveries, jobs,
    profiles
    to keel_app;

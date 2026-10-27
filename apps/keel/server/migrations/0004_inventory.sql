-- Inventories and their immutable revisions (spec 055 US4, T056; data-model.md "Inventory").

create table inventories (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    name text not null check (name ~ '^[a-z0-9][a-z0-9_-]{0,62}$'),
    source text not null check (source in ('repo', 'managed')),
    binding_id uuid references repo_bindings (id) on delete set null,
    path text check (path is null or char_length(path) between 1 and 1024),
    var_schema jsonb,
    status text not null default 'active' check (status in ('active', 'deprecated')),
    created_at timestamptz not null default now(),
    unique (workspace_id, name),
    check (source <> 'repo' or path is not null)
);

-- Immutable: keel_app may insert and read, never update or delete (below).
create table inventory_revisions (
    id uuid primary key,
    inventory_id uuid not null references inventories (id) on delete cascade,
    seq integer not null check (seq >= 1),
    parent_id uuid references inventory_revisions (id),
    -- Normalized hosts, groups and vars.
    content jsonb not null,
    content_hash text not null,
    source_commit text,
    author_user_id uuid references users (id) on delete set null,
    restored_from uuid references inventory_revisions (id),
    imported_from jsonb,
    status text not null check (status in ('valid', 'invalid')),
    errors jsonb not null default '[]',
    warnings jsonb not null default '[]',
    created_at timestamptz not null default now(),
    unique (inventory_id, seq)
);

-- Derived per revision, for search and target resolution.
create table inventory_hosts (
    revision_id uuid not null references inventory_revisions (id) on delete cascade,
    host text not null,
    groups text[] not null default '{}',
    primary key (revision_id, host)
);

grant select, insert, update, delete on inventories to keel_app;
grant select, insert on inventory_revisions, inventory_hosts to keel_app;
revoke update, delete, truncate on inventory_revisions, inventory_hosts from keel_app;

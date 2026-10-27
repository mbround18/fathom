-- Spec 055 T021: identity, access and the audit log.
--
-- Migrations are additive only: no DROP, no type change. Tables live in the connection's
-- current schema (tests use one schema each).
--
-- The server runs migrations as the database owner, then serves as the NOLOGIN role `keel_app`
-- (`SET ROLE`, db.rs), which holds SELECT and INSERT only on `audit_event`. A trigger refuses
-- UPDATE, DELETE and TRUNCATE on `audit_event` for everyone, the owner included.

create table users (
    id uuid primary key,
    issuer text not null,
    subject text not null,
    display_name text not null,
    email text,
    instance_role text not null check (instance_role in ('owner', 'admin', 'user')),
    authz_epoch bigint not null default 1,
    disabled_at timestamptz,
    created_at timestamptz not null default now(),
    unique (issuer, subject)
);

-- `id` is the SHA-256 of the session cookie (web-auth's session id), never the cookie itself.
create table sessions (
    id text primary key,
    user_id uuid not null references users (id),
    authz_epoch bigint not null,
    verified_at timestamptz,
    created_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now(),
    expires_at timestamptz not null
);
create index sessions_user on sessions (user_id);

-- The 054 passkey shape, moved from files to tables (crates/web-passkeys owns the logic).
create table passkeys (
    id text primary key,                 -- credential id, base64url
    user_id uuid not null references users (id),
    name text not null check (char_length(name) between 1 and 64),
    credential jsonb not null,           -- public key material only
    created_at timestamptz not null default now(),
    last_used_at timestamptz,
    unique (user_id, name)
);

-- At most one code per user; a new one replaces the old. Only the SHA-256 is stored.
create table passkey_recovery (
    user_id uuid primary key references users (id),
    code_sha256 bytea not null,
    expires_at timestamptz not null,
    used_at timestamptz
);

create table workspaces (
    id uuid primary key,
    slug text not null unique check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    name text not null,
    created_at timestamptz not null default now(),
    archived_at timestamptz
);

create table projects (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id),
    slug text not null check (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    name text not null,
    unique (workspace_id, slug)
);

create table teams (
    id uuid primary key,
    name text not null unique,
    origin text not null default 'local' check (origin in ('local'))
);

create table team_members (
    team_id uuid not null references teams (id),
    user_id uuid not null references users (id),
    origin text not null default 'local' check (origin in ('local')),
    primary key (team_id, user_id)
);
create index team_members_user on team_members (user_id);

create table grants (
    id uuid primary key,
    team_id uuid not null references teams (id),
    workspace_id uuid not null references workspaces (id),
    role text not null check (role in ('viewer', 'operator', 'approver', 'ws_admin')),
    profiles text[] not null default '{}',
    origin text not null default 'local' check (origin in ('local'))
);
create index grants_team on grants (team_id);
create index grants_workspace on grants (workspace_id);

-- `hash` is the SHA-256 of the token; the token is shown once and never stored.
create table api_tokens (
    id uuid primary key,
    name text not null,
    owner_user_id uuid not null references users (id),
    workspace_id uuid not null references workspaces (id),
    actions text[] not null,
    hash bytea not null unique,
    expires_at timestamptz not null,
    last_used_at timestamptz,
    revoked_at timestamptz
);

-- Insert-only. `seq` orders the hash chain; `hash = SHA-256(prev_hash || canonical row)`
-- (audit.rs). `actor` is a stable key (`user:<uuid>`, `system`, `receiver:<id>`, `host:<id>`)
-- and `actor_detail` holds the issuer, subject and display name for users.
create table audit_event (
    seq bigint generated always as identity primary key,
    id uuid not null unique,
    at timestamptz not null,
    actor text not null,
    actor_detail jsonb not null default '{}',
    action text not null,
    workspace_id uuid,
    target jsonb not null default '{}',
    decision text not null check (decision in ('allowed', 'denied')),
    reason text,
    context jsonb not null default '{}',
    prev_hash bytea not null check (octet_length(prev_hash) = 32),
    hash bytea not null unique check (octet_length(hash) = 32)
);
create index audit_event_at on audit_event (at);
create index audit_event_workspace_at on audit_event (workspace_id, at);
create index audit_event_actor_at on audit_event (actor, at);
create index audit_event_action_at on audit_event (action, at);

create function audit_event_refuse_change() returns trigger
language plpgsql as $$
begin
    raise exception 'audit_event is insert-only (% refused)', tg_op
        using errcode = 'insufficient_privilege';
end;
$$;

create trigger audit_event_no_update_delete
    before update or delete on audit_event
    for each row execute function audit_event_refuse_change();

create trigger audit_event_no_truncate
    before truncate on audit_event
    for each statement execute function audit_event_refuse_change();

-- The application role. Created once per cluster; tolerate a concurrent creator.
do $$
begin
    if not exists (select 1 from pg_roles where rolname = 'keel_app') then
        begin
            create role keel_app nologin;
        exception when duplicate_object or unique_violation then
            null;
        end;
    end if;
    if not pg_has_role(current_user, 'keel_app', 'MEMBER') then
        execute format('grant keel_app to %I', current_user);
    end if;
end;
$$;

do $$
begin
    execute format('grant usage on schema %I to keel_app', current_schema());
    execute format(
        'grant select, insert, update, delete on all tables in schema %I to keel_app',
        current_schema()
    );
end;
$$;
revoke all on audit_event from public;
revoke update, delete, truncate on audit_event from keel_app;
grant select, insert on audit_event to keel_app;
revoke all on _sqlx_migrations from keel_app;

-- Instance settings, enrolment revoke and roll, OIDC-managed roles and the owner-only System
-- workspace (spec 060, T003; data-model.md, research R1, R4, R7, R8). Additive only.

-- At most one row. With no row the defaults apply (the default template, empty instructions,
-- version 0). A save bumps `version`; a save against a stale version is a conflict.
create table instance_settings (
    id boolean primary key default true check (id),
    enroll_template text check (octet_length(enroll_template) <= 4096),
    enroll_instructions text not null default '' check (octet_length(enroll_instructions) <= 20480),
    version bigint not null default 1,
    updated_by uuid references users (id) on delete set null,
    updated_at timestamptz not null default now()
);

-- A rolled token points at the one it replaced. Rows are never deleted.
alter table host_enrollments add column rolled_from uuid references host_enrollments (id);

-- Whether a user's instance role is Keel's to change, or comes from the identity provider's
-- claims. `idp_role` is the last role the mapping produced.
alter table users add column role_source text not null default 'keel'
    check (role_source in ('keel', 'idp'));
alter table users add column idp_role text check (idp_role in ('owner', 'admin', 'user'));

-- The System workspace (slug `keel-system`): at most one, instance owners only.
alter table workspaces add column system boolean not null default false;
create unique index workspaces_one_system on workspaces (system) where system;

-- At most one row. No row means no System workspace as far as the UI is concerned.
create table system_config (
    id boolean primary key default true check (id),
    workspace_id uuid not null references workspaces (id),
    allowed_profiles text[] not null check (cardinality(allowed_profiles) between 1 and 16),
    version bigint not null default 1,
    updated_by uuid references users (id) on delete set null,
    updated_at timestamptz not null default now()
);

grant select, insert, update on instance_settings, system_config to keel_app;
grant delete on system_config to keel_app;
revoke delete, truncate on instance_settings from keel_app;
revoke truncate on system_config from keel_app;

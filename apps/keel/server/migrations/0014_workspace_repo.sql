-- Spec 066: workspaces, the primary repository, forge connections, catalogs and host age keys.
-- Additive only (data-model.md).

-- R3: a workspace's primary repository binding. Removing the binding clears it; nothing promotes
-- another binding silently (spec 066 US2 scenario 5).
alter table workspaces
    add column primary_binding uuid references repo_bindings (id) on delete set null,
    add column primary_set_by uuid references users (id) on delete set null,
    add column primary_set_at timestamptz;

-- R2: every workspace has a `default` project, so a first binding needs no detour through Teams.
insert into projects (id, workspace_id, slug, name)
select gen_random_uuid(), w.id, 'default', 'Default'
  from workspaces w
 where not exists (select 1 from projects p where p.workspace_id = w.id);

-- Backfill: each workspace's oldest active binding becomes its primary.
update workspaces w
   set primary_binding = b.id, primary_set_at = now()
  from (select distinct on (p.workspace_id) p.workspace_id, rb.id
          from repo_bindings rb
          join projects p on p.id = rb.project_id
         where rb.status = 'active'
         order by p.workspace_id, rb.created_at) b
 where b.workspace_id = w.id
   and w.primary_binding is null;

-- FR-008: a forge API token, used only to open pull requests on a new branch.
alter table credentials drop constraint credentials_kind_check;
alter table credentials add constraint credentials_kind_check check (
    kind in ('vault_password', 'env', 'secret_vars', 'git_https_token', 'git_ssh_key', 'forge_token')
);

-- FR-008: at most one forge connection per workspace.
create table forge_connections (
    workspace_id uuid primary key references workspaces (id) on delete cascade,
    provider text not null check (provider in ('forgejo', 'gitea', 'github')),
    api_base text not null check (char_length(api_base) between 8 and 2048),
    credential_id uuid not null references credentials (id) on delete restrict,
    probed_at timestamptz,
    probe_error text,
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now()
);

-- FR-009: one catalog per binding, replaced on each build; a failed build marks it stale and keeps
-- the last good document. Never holds a sops value (FR-015).
create table binding_catalogs (
    binding_id uuid primary key references repo_bindings (id) on delete cascade,
    commit text not null,
    catalog jsonb not null,
    built_at timestamptz not null default now(),
    stale boolean not null default false,
    error text
);

-- R7: the host's age public key, as its runner last reported it.
alter table hosts add column age_recipient text
    check (age_recipient is null or age_recipient ~ '^age1[02-9ac-hj-np-z]{58}$');

grant select, insert, update, delete on forge_connections, binding_catalogs to keel_app;
revoke truncate on forge_connections, binding_catalogs from keel_app;

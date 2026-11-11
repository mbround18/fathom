# Data model: Keel workspaces, repository catalog and hosts

One additive migration, `apps/keel/server/migrations/0014_workspace_repo.sql`.

## Changes

```sql
-- R3: a workspace's primary repository binding.
alter table workspaces
    add column primary_binding uuid references repo_bindings (id) on delete set null,
    add column primary_set_by uuid references users (id) on delete set null,
    add column primary_set_at timestamptz;

-- R2: every workspace has a `default` project.
insert into projects (id, workspace_id, slug, name)
select gen_random_uuid(), w.id, 'default', 'Default'
  from workspaces w
 where not exists (select 1 from projects p where p.workspace_id = w.id);

-- Backfill: the oldest active binding becomes primary.
update workspaces w set primary_binding = b.id, primary_set_at = now()
  from (select distinct on (p.workspace_id) p.workspace_id, rb.id
          from repo_bindings rb join projects p on p.id = rb.project_id
         where rb.status = 'active'
         order by p.workspace_id, rb.created_at) b
 where b.workspace_id = w.id and w.primary_binding is null;

-- FR-008: one forge connection per workspace.
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

-- A new credential kind for forge API tokens.
alter table credentials drop constraint credentials_kind_check;
alter table credentials add constraint credentials_kind_check check (
    kind in ('vault_password', 'env', 'secret_vars', 'git_https_token', 'git_ssh_key', 'forge_token')
);

-- FR-009: one catalog per binding, replaced on each build.
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
```

`gen_random_uuid()` is core PostgreSQL 13 and later. Before writing the migration, check whether
an earlier migration already uses it. If none does, generate UUIDv7 in Rust at create time, and
in the migration use `md5(random()::text || w.id::text)::uuid` (it doesn't matter for a default
row).

The name of the credentials check constraint must be confirmed against
`0003_bindings.sql` (`credentials_kind_check` is PostgreSQL's default name for an inline column
check).

## Catalog document (`binding_catalogs.catalog`)

This is serialised from `keel_core::catalog::Catalog`, and the protocol type mirrors it.

```text
Catalog {
  version: 1,
  commit,
  playbooks: [{ path, plays: [{ name?, hosts }], roles: [name], imports: [path],
                vars_files: [path], unresolved: [string], jobs: [job name] }],
  roles: [{ name, path, description?, defaults: [var name], dependencies: [name],
            reached_by: [playbook path], external: bool }],
  inventories: [{ path, kind: yaml|ini|dir|dynamic, groups: [{ name, parent?, hosts: [name],
                  children: [name], vars_file? }] }],
  secrets: { rules_file?: path, rules: [{ index, path_regex, age: [key], pgp: [fp] }],
             files: [{ path, rule?: index, age: [key], pgp: [fp], lastmodified?, keys: [dotted],
                       drift: { missing: [key], extra: [key] } | null, problem?,
                       read_by: [job name] }] },
  problems: [{ path, line?, message }],
  skipped: { files, bytes, reason? }
}
```

Hosts' decrypt state is not stored. It is computed when the catalog is read, from
`hosts.age_recipient` and the catalog, so it updates as soon as a runner reports a new key.

## State: host inventory status (derived)

`listed`: the primary catalog's inventories list `hosts.inventory_name`.

`pending_inventory`: they don't.

`unknown`: there is no primary catalog.

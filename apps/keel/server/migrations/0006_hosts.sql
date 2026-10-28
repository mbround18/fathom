-- Host enrollment, SSH CAs and runner certificates (spec 055 US7, T080; data-model.md "Hosts",
-- research R17, contracts/runner.md). 0005_runs.sql created the minimal `hosts` and
-- `host_profiles` runs need; this migration only adds to them, so runs keep working.

-- What a runner reported in its last `Hello`.
alter table hosts add column ledger_head text;
alter table hosts add column python_version text;
alter table hosts add column override_quorum integer;

-- One-time enrollment tokens. Only the SHA-256 of the whole token is kept; the token itself is
-- shown once, when the preview is confirmed. A token is spent by use or burned by any failed
-- attempt (`used_at` set either way, `burned_reason` says which).
create table host_enrollments (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    inventory_name text not null check (char_length(inventory_name) between 1 and 253),
    profiles text[] not null default '{}',
    token_hash bytea not null unique check (octet_length(token_hash) = 32),
    expires_at timestamptz not null,
    used_at timestamptz,
    burned_reason text,
    host_id uuid references hosts (id) on delete set null,
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now()
);
create index host_enrollments_workspace on host_enrollments (workspace_id, created_at);

-- The two SSH CAs (host: Keel's listener identity; runner: runner certificates). The private
-- key is sealed with the master key under AAD `ssh_ca:<id>`. One active CA per kind; a retiring
-- CA stays trusted until `not_after` (the expiry of the last certificate it signed).
create table ssh_cas (
    id uuid primary key,
    kind text not null check (kind in ('host', 'runner')),
    public_key text not null,
    fingerprint text not null unique,
    sealed_private_key bytea not null,
    state text not null default 'active' check (state in ('active', 'retiring', 'retired')),
    not_after timestamptz,
    created_at timestamptz not null default now()
);
create unique index ssh_cas_one_active on ssh_cas (kind) where state = 'active';

-- Every runner certificate issued. Serials are u64 stored bit-for-bit in a bigint. The KRL is
-- built from the rows with `revoked_at` set whose `valid_before` is still ahead.
create table ssh_certs (
    id uuid primary key,
    serial bigint not null unique,
    ca_id uuid not null references ssh_cas (id),
    host_id uuid not null references hosts (id) on delete cascade,
    principals text[] not null,
    valid_after timestamptz not null,
    valid_before timestamptz not null,
    revoked_at timestamptz,
    created_at timestamptz not null default now()
);
create index ssh_certs_host on ssh_certs (host_id, valid_before);
create index ssh_certs_revoked on ssh_certs (valid_before) where revoked_at is not null;

grant select, insert, update on host_enrollments, ssh_cas, ssh_certs to keel_app;
revoke delete, truncate on host_enrollments, ssh_cas, ssh_certs from keel_app;

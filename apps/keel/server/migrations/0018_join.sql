-- Join tokens and join requests (spec 055 US7 follow-up, T144; contracts/join.md,
-- data-model.md). Additive only. Numbered after 0017 (0015 is reserved for another branch).

-- Keel's join signing key: an Ed25519 key that signs join tokens and nothing else. The private
-- seed is sealed with the master key under AAD `join_signing_key:<kid>`. One unretired row;
-- rotating retires it, and with it every token it signed. Retired, never deleted.
create table join_signing_key (
    kid text primary key check (kid ~ '^[0-9a-f]{32}$'),
    public_key bytea not null check (octet_length(public_key) = 32),
    sealed_private_key bytea not null,
    created_at timestamptz not null default now(),
    retired_at timestamptz
);
create unique index join_signing_key_one_active on join_signing_key ((true))
    where retired_at is null;

-- Join tokens: only the `jti` and the SHA-256 of the whole token are kept. At most one active
-- (unretired) token per workspace; none is the default.
create table join_tokens (
    jti text primary key check (jti ~ '^[0-9a-f]{32}$'),
    workspace_id uuid not null references workspaces (id) on delete cascade,
    kid text not null references join_signing_key (kid),
    token_hash bytea not null unique check (octet_length(token_hash) = 32),
    inventory_name text check (char_length(inventory_name) between 1 and 253),
    profiles text[] not null default '{}',
    expires_at timestamptz not null,
    created_by uuid references users (id) on delete set null,
    created_at timestamptz not null default now(),
    retired_at timestamptz
);
create unique index join_tokens_one_active on join_tokens (workspace_id) where retired_at is null;

-- One request per (workspace, host key). Everything but the key is the host's claim, bounded
-- here as in the frame. Pending rows expire lazily after 7 days; a rejected fingerprint is
-- answered `JoinRejected` for 30 days. Accepting creates the `hosts` row; the certificate is
-- issued at the runner's next attempt.
create table join_requests (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    jti text not null references join_tokens (jti),
    ssh_key_fingerprint text not null check (char_length(ssh_key_fingerprint) <= 128),
    ssh_public_key text not null check (char_length(ssh_public_key) <= 512),
    inventory_name text not null check (char_length(inventory_name) <= 253),
    allowed_profiles text[] not null default '{}',
    require_signed boolean not null,
    hostname text not null check (char_length(hostname) <= 253),
    os text not null check (char_length(os) <= 128),
    arch text not null check (arch in ('x86_64', 'aarch64')),
    runner_version text not null check (char_length(runner_version) <= 64),
    source_addr text not null check (char_length(source_addr) <= 300),
    state text not null default 'pending'
        check (state in ('pending', 'accepted', 'rejected', 'expired')),
    reason text check (char_length(reason) <= 64),
    comment text check (char_length(comment) <= 500),
    first_seen_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now(),
    decided_by uuid references users (id) on delete set null,
    decided_at timestamptz,
    host_id uuid references hosts (id) on delete set null,
    unique (workspace_id, ssh_key_fingerprint)
);
create index join_requests_workspace_state on join_requests (workspace_id, state, last_seen_at);
create index join_requests_jti on join_requests (jti) where state = 'pending';
create index join_requests_fingerprint on join_requests (ssh_key_fingerprint);

grant select, insert, update on join_signing_key, join_tokens, join_requests to keel_app;
revoke delete, truncate on join_signing_key, join_tokens, join_requests from keel_app;

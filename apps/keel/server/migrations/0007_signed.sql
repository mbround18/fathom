-- Signed mode with reviewed overrides (spec 055 US8, T099; data-model.md "Runs" and "Hosts",
-- research R19 and R20, contracts/signed-mode.md). Additive only: earlier tables are untouched.

-- One per distinct host base in a signed-mode run that fails Keel's early signature check. The
-- check is advisory: hosts verify on their own and refuse what they cannot verify.
create table review_groups (
    id uuid primary key,
    run_id uuid not null references runs (id) on delete cascade,
    -- Null for a baseline (no verified base on these hosts).
    base_commit text,
    target_commit text not null,
    hosts text[] not null check (cardinality(hosts) >= 1),
    -- Per commit: id, author, committer, signature status, problem and signer.
    commits jsonb not null default '[]',
    -- Trust and targeting changes (`.keel/`, inventories, `keel_runner_*`), R20.
    flags jsonb not null default '[]',
    state text not null default 'open' check (state in ('open', 'approved', 'denied', 'expired')),
    expires_at timestamptz,
    created_at timestamptz not null default now()
);
create index review_groups_run on review_groups (run_id);
create index review_groups_open on review_groups (state) where state = 'open';

-- Approvals of a run (`approval`) and reviewed overrides of a review group (`override`).
-- `override` rows carry the comment and the passkey assertion over the canonical challenge;
-- they are sent to hosts unchanged. The challenge itself is never stored, only its SHA-256.
create table approvals (
    id uuid primary key,
    run_id uuid not null references runs (id) on delete cascade,
    approver_user_id uuid not null references users (id),
    kind text not null check (kind in ('approval', 'override')),
    decision text not null check (decision in ('approve', 'deny')),
    comment text check (char_length(comment) <= 2000),
    at timestamptz not null default now(),
    step_up_at timestamptz,
    review_group_id uuid references review_groups (id) on delete cascade,
    challenge_sha256 text check (challenge_sha256 ~ '^[0-9a-f]{64}$'),
    -- Credential id, authenticator data, client data JSON and signature, base64url.
    assertion jsonb,
    expires_at timestamptz,
    unique nulls not distinct (run_id, approver_user_id, kind, review_group_id),
    -- An approved override always has its group, comment, challenge and assertion (R20).
    check (
        kind <> 'override' or decision <> 'approve' or (
            review_group_id is not null and comment is not null and char_length(comment) >= 1
            and challenge_sha256 is not null and assertion is not null and expires_at is not null
        )
    )
);
create index approvals_run on approvals (run_id);

-- Keel's copy of the signers it trusts, for the early check only. Hosts pin their own (R19).
create table trusted_signers (
    id uuid primary key,
    workspace_id uuid not null references workspaces (id) on delete cascade,
    kind text not null check (kind in ('openpgp', 'ssh')),
    fingerprint text not null check (char_length(fingerprint) between 1 and 128),
    public_key text not null check (char_length(public_key) between 1 and 65536),
    label text not null check (char_length(label) between 1 and 128),
    -- For four-eyes: the person whose key this is.
    user_id uuid references users (id) on delete set null,
    added_by uuid references users (id) on delete set null,
    added_at timestamptz not null default now(),
    removed_at timestamptz,
    unique (workspace_id, fingerprint)
);

-- One row per host run, from the ledger line the host reported (R19). The latest row per
-- (`host_id`, `job_id`) is that host's base. Insert-only. `unchecked` marks a run outside
-- signed mode, which the host does not authorize against signatures. (`authorization` is a
-- reserved word in SQL, hence `authorization_kind`.)
create table host_revisions (
    id uuid primary key,
    host_id uuid not null references hosts (id) on delete cascade,
    job_id uuid references jobs (id) on delete set null,
    commit text not null,
    run_id uuid references runs (id) on delete set null,
    authorization_kind text not null
        check (authorization_kind in ('signed', 'override', 'baseline', 'unchecked')),
    signers text[] not null default '{}',
    approval_ids uuid[] not null default '{}',
    ledger_hash text,
    applied_at timestamptz not null default now(),
    outcome text not null check (outcome in ('succeeded', 'failed'))
);
create index host_revisions_base on host_revisions (host_id, job_id, applied_at desc);

grant select, insert, update, delete on review_groups, trusted_signers to keel_app;
-- Approvals and host revisions are records: keel_app may add and read them, never change them.
grant select, insert on approvals, host_revisions to keel_app;
revoke update, delete, truncate on approvals, host_revisions from keel_app;

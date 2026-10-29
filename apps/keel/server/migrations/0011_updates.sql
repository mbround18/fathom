-- Spec 061: update notice and self-update (T016; data-model.md, research R4, R6). Additive only.

-- The update digest an owner dismissed the navigation dot for (R6). Null means none.
alter table users add column update_dot_dismissed text
    check (update_dot_dismissed ~ '^sha256:[0-9a-f]{64}$');

-- Values for parameters the job's manifest declares (R4). Empty for every run before this spec.
alter table runs add column params jsonb not null default '{}'::jsonb
    check (jsonb_typeof(params) = 'object');

-- At most one row: the build Keel last started as, so a start on a different digest or revision
-- can be audited as `keel_updated` (FR-019).
create table keel_build_seen (
    id boolean primary key default true check (id),
    digest text check (digest ~ '^sha256:[0-9a-f]{64}$'),
    revision text,
    version text,
    seen_at timestamptz not null default now()
);

grant select, insert, update on keel_build_seen to keel_app;
revoke delete, truncate on keel_build_seen from keel_app;

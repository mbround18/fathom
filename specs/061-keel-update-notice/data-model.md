# Data model: Keel update notice and self-update (spec 061)

There is one migration, `apps/keel/server/migrations/0011_updates.sql`, and it is additive only.
Everything about checks lives in memory (FR-008).

## Migration 0011

```sql
-- Spec 061: update notice and self-update (research R4, R6). Additive only.

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
```

`users` and `runs` keep their existing grants.

## Running build (in memory, fixed for the process)

| Field | Type | Source | Rules |
|---|---|---|---|
| `version` | `Option<String>` | `KEEL_BUILD_VERSION` | Semver without the `keel-v` prefix; empty is `None` |
| `revision` | `Option<String>` | `KEEL_BUILD_REVISION` | 7–40 hex characters, optional `-dirty` |
| `built_at` | `Option<OffsetDateTime>` | `KEEL_BUILD_DATE` | RFC 3339 |
| `source` | `Option<Url>` | `KEEL_BUILD_SOURCE` | `https` only |
| `digest` | `Option<String>` | `KEEL_IMAGE_DIGEST` | `sha256:` and 64 lowercase hex characters |
| `platform` | `(os, arch, variant)` | `std::env::consts` | Used to pick an index entry (R1) |

A malformed value stops Keel at start, naming the variable. A missing value is `None`, shown as
"unknown".

## Update status (in memory, `watch` channel)

| Field | Type | Rules |
|---|---|---|
| `enabled` | `bool` | `KEEL_UPDATE_CHECK` |
| `image` | `String` | Watched reference, without credentials |
| `state` | `not_checked` / `checking` / `checked` | Starts at `not_checked` after every restart (FR-008) |
| `outcome` | `up_to_date` / `update_available` / `older_build` / `rebuilt` / `cant_tell` | From the last successful check |
| `cant_tell_reason` | `Option<String>` | e.g. `no_image_for_platform`, `missing_labels`, `running_unknown` |
| `checked_at` | `Option<OffsetDateTime>` | Last successful check |
| `available` | `Option<ImageInfo>` | Set unless `up_to_date` |
| `last_error` | `Option<{ code, at }>` | The last failure since the last success. Cleared on success (FR-010). `code` is `RegistryError::code()`: `auth_failed`, `not_found`, `rate_limited`, `timeout`, `network`, `bad_response` or `no_digest_header` |
| `next_check_at` | `Option<OffsetDateTime>` | |
| `manual_after` | `Option<OffsetDateTime>` | When Check now is allowed again (FR-007) |
| `digest_compare` | `bool` | False when the running digest is unknown (R3) |

`ImageInfo` holds `{ digest, version, revision, built_at, release_notes: Option<Url> }`. It is
built from `oci_registry::Resolved` (contracts/registry.md): `digest` is the HEAD digest of the
tag, and `version`, `revision` and `built_at` come from the `org.opencontainers.image.*` labels,
with `Resolved.created` as the fallback date. `NoImageForPlatform` becomes `cant_tell` with
`no_image_for_platform`.

Remote-digest cache: `HashMap<String, ImageInfo>`, capped at 16 entries and evicting the oldest.
A cache hit makes no request beyond the HEAD.

The admins' `UpdateStatusLine` is derived from this status and carries only `update_available`
and `checked_at` (SC-006).

## Dot dismissal

`users.update_dot_dismissed`. Only owners can write it. The value is the digest that was available
when they dismissed. The dot shows when:

```text
role == owner
  && status.outcome == update_available
  && status.available.digest != me.update_dot_dismissed
```

The card is not affected (FR-014).

## Update request

There is no table of its own. An update request is:

- a row in `runs`:
  - `trigger = 'manual'`, `requested_by` = the owner;
  - `job` = the `keel-server` job of the System workspace binding;
  - `params = {"keel_server_image_digest": "<new>"}`;
- the audit entries below.

"Only one at a time" (FR-018) is this query, under the same transaction as the insert:

```sql
select id from runs
 where workspace_id = $system
   and params ? 'keel_server_image_digest'
   and state in ('pending_approval', 'queued', 'running')
 for update
```

## Job parameters (keel-core manifest)

| Field | Type | Rules |
|---|---|---|
| `params` | `BTreeMap<String, ParamSpec>` | Optional; at most 8 entries |
| name | `String` | `^[a-z_][a-z0-9_]{0,63}$`; not `ansible_*` or `keel_internal_*` |
| `ParamSpec.kind` | `oci_digest` | Closed enum. `oci_digest` is `^sha256:[0-9a-f]{64}$` |

The name may not also appear in the job's `vars` (refused at manifest parse).

## Audit actions

| Action | When | Detail |
|---|---|---|
| `keel_update_requested` | Confirm of `PreviewKeelUpdate` | `old_digest`, `new_digest`, `old_revision`, `new_revision`, `run`, `workspace` |
| `keel_update_refused` | Any refusal of `PreviewKeelUpdate` or its confirm | `reason` (`owner_only`, `token_caller`, `no_system_workspace`, `no_keel_host`, `profile_not_allowed`, `job_missing_param`, `digest_stale`, `update_in_progress`), plus `old_digest` and `new_digest` when known |
| `keel_update_result` | The update run finishes (success, failure, cancelled or `runner_lost`) | `run`, `outcome`, `old_digest`, `new_digest` |
| `keel_updated` | Keel starts with a digest or revision different from `keel_build_seen` | `old_digest`, `new_digest`, `old_revision`, `new_revision`, `old_version`, `new_version`; actor `internal()` |
| `update_check_failed` | Not audited | Checks are reads; failures go to logs, the status and telemetry |
| `update_dot_dismissed` | Not audited | A display preference |

`keel_updated` is written once per change. On the first start after this migration, the row is
filled without an audit entry, because there is nothing to compare against.

## Changed records

- **`Me`** gains:
  - `keel_version: Option<String>`, for everyone;
  - `update_dot_dismissed: Option<String>`, for owners only, null for others.
- **`RunInfo`** gains `params: BTreeMap<String, String>`. It is empty for older runs.

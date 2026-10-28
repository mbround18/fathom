# Data model: spec 060

All changes are additive, in migration `apps/keel/server/migrations/0010_settings_system.sql`. The
`keel_app` role gets `select, insert, update` on the new tables, and no `delete` or `truncate`,
except where noted.

## instance_settings (new, at most one row)

| Column | Type | Rules |
|---|---|---|
| id | boolean | primary key, `default true`, `check (id)` |
| enroll_template | text | `check (octet_length(enroll_template) <= 4096)`; must contain `%t` (checked in code: FR-009) |
| enroll_instructions | text | `not null default ''`, `check (octet_length(enroll_instructions) <= 20480)` |
| version | bigint | `not null default 1`; bumped on every save |
| updated_by | uuid | references `users (id)` `on delete set null` |
| updated_at | timestamptz | `not null default now()` |

- **No row**: the default template, `read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t\nmake
  keel-node`, with empty instructions and `version = 0`.
- **Saving**: the first save inserts the row. Later saves run `update … where version = $loaded`,
  and zero rows updated means a conflict (FR-014).
- **Restore default**: saves the default text as a normal audited change.

## host_enrollments (changed)

- **Added column**: `rolled_from uuid references host_enrollments (id)`.
- **Derived state**, in this order: used, revoked, burned, expired, pending (research R4).
- **Revoke**: sets `used_at` and `burned_reason = 'revoked'`.
- **Roll**: inserts a row with `rolled_from`.
- **No deletes**: rows are never deleted.

## users (changed)

| Column | Type | Rules |
|---|---|---|
| role_source | text | `not null default 'keel'`, `check (role_source in ('keel', 'idp'))` |
| idp_role | text | `check (idp_role in ('owner', 'admin', 'user'))`; the last role the mapping produced, or null |

- **Mapped sign-in**: sets `role_source = 'idp'`.
- **Mapping removed**: rows keep `'idp'` until an admin changes the role. That change sets them
  back to `'keel'`, and is allowed only while no mapping is active.

## workspaces (changed)

- **Added column**: `system boolean not null default false`.
- **Index**: `create unique index workspaces_one_system on workspaces (system) where system`.
- **Reserved slug**: `keel-system`, refused by workspace create.

## system_config (new, at most one row)

| Column | Type | Rules |
|---|---|---|
| id | boolean | primary key, `default true`, `check (id)` |
| workspace_id | uuid | `not null`, references `workspaces (id)` |
| allowed_profiles | text[] | `not null`, 1 to 16 entries, each a profile name |
| version | bigint | `not null default 1` |
| updated_by | uuid | references `users (id)` `on delete set null` |
| updated_at | timestamptz | `not null default now()` |

- **Removal**: deletes the row. `keel_app` gets `delete` on this table only.
- **No row**: there is no System workspace, as far as the UI is concerned (FR-039).

## Audit actions (new)

All of these use the existing `audit::record`, and none carries a token value or hash.

| Action | Target | Context |
|---|---|---|
| `settings_update` | `{}` | `{before: {template, instructions}, after: {…}, version}` |
| `host_enrollment_revoked` | `{enrollment, inventory_name}` | `{workspace, by_admin}` |
| `host_enrollment_created` (existing) | as today | adds `rolled_from` when rolled |
| `api_token_revoke` (existing) | as today | adds `by_admin: true` when the revoker isn't the owner |
| `user_role` (existing) | `{user, from, to}` | reason `identity_provider`, or `idp_names_no_owner` |
| `sign_in` (refused) | `{}` | reason `role_not_mapped` |
| `system_configure` | `{workspace}` | `{allowed_profiles, before}` |
| `system_remove` | `{workspace}` | `{hosts_revoked, bindings_disabled}` |
| `run_result_late` | `{run, host}` | `{outcome}` |

## Runner state (host side)

- **Undelivered results**: `<state_dir>/undelivered/<run_id>.json` holds one serialised `Finished`
  frame, mode 0600.
  - It is deleted once the server acknowledges the frame.
  - It is dropped once it is older than the maximum offline time.

# Research: Keel Enroll, System Settings, admin tokens, OIDC roles, System workspace and polish

Each decision below resolves an open point in [plan.md](plan.md).

## R1. Where instance settings live

- **Decision**: A single-row table `instance_settings` (`id boolean primary key default true check (id)`),
  holding:
  - `enroll_template` and `enroll_instructions`;
  - a `version bigint` for optimistic concurrency;
  - `updated_by` and `updated_at`.

  With no row, the defaults apply, so a fresh instance works without a migration seeding text.
- **Rationale**: The settings are relational, and there is exactly one set of them. A version column
  gives FR-014's "refused if changed since loaded" with one `update … where version = $n`.
- **Alternatives**:
  - A key-value `settings` table: loses column checks for the 4 KB and 20 KB caps.
  - A file in the data dir: neither audited nor transactional with the audit row.

## R2. Rendering the snippet template

- **Decision**:
  - **Server**: `keel-core` gets a pure `enroll_template` module with `validate(template, ctx)` and
    `render(template, values)`. The server is the authority. It validates on save, and renders the
    real snippet into `Event::HostEnrollment.snippet` when a token is minted.
  - **Web**: a twin of about 40 lines (`features/settings/template.ts`) renders the live preview
    only.
  - **Shared tests**: both are tested against the same vector file,
    `specs/060-keel-enroll-settings/contracts/template-vectors.json`.
- **Rationale**:
  - The real token never has to be substituted client-side from a template the client could
    have tampered with.
  - The preview updates on every keystroke without a round trip.
  - The shared vectors stop the two renderers drifting.
- **Single pass (FR-010)**: The renderer scans the template once, left to right, emitting either
  literal bytes or a substituted value. Values are never rescanned.
- **Alternatives**:
  - A server-only preview command: a round trip per keystroke, and preview breaks offline.
  - A client-only renderer: the server couldn't validate, and the snippet logic would sit in the
    browser.

## R3. Rendering Markdown safely

- **Decision**: `react-markdown` + `remark-gfm`, loaded with `React.lazy` in its own chunk
  (`features/settings/Markdown.tsx`). Approved in conversation. It is configured as follows:
  - **Raw HTML as text**: a 10-line remark plugin turns `html` nodes into `text` nodes. Without it,
    react-markdown silently drops raw HTML. The spec wants it shown as text, so the admin sees what
    they typed.
  - **Links**: `components.a` sets `target="_blank" rel="noopener noreferrer"`. The default
    `urlTransform` already strips `javascript:` URLs.
  - **Images**: `components.img` renders a plain link to the image's URL with its alt text. Nothing
    is fetched.
- **Rationale**: Uses the standard GFM support: tables, task lists, fenced code. The safety comes
  from the configuration above, not from sanitising HTML afterwards.
- **Alternatives**:
  - `marked` + DOMPurify: two dependencies, and an HTML sanitiser to keep current.
  - `rehype-sanitize`: unnecessary when no HTML is ever parsed.

## R4. Enrolment token states, revoke and roll

- **Decision**: The state is derived from the existing columns, plus a new `rolled_from uuid`
  column:

  | State | Condition |
  |---|---|
  | used | `host_id is not null` |
  | revoked | `burned_reason = 'revoked'` |
  | burned | `used_at is not null` (any other reason) |
  | expired | `expires_at <= now()` and `used_at is null` |
  | pending | none of the above |

  - **Revoke**: `update … set used_at = now(), burned_reason = 'revoked' where id = $1 and used_at
    is null`. This uses the token-spending path that already exists, so the runner sees revoked
    tokens exactly as it sees burned ones (FR-017's "same response").
  - **Roll**: one transaction that does the following:
    - revokes the old token if it's still pending;
    - re-runs `check_enroll`, so the host name must still be free;
    - inserts the new token with `rolled_from = old.id`;
    - writes two audit rows, `host_enrollment_revoked` and `host_enrollment_created`, where the
      second's context carries `rolled_from`.
- **Rationale**: No new state column to keep in sync. The hash-only rule (FR-022) is unchanged.
- **Alternatives**: An explicit `state` enum column. That is redundant with the timestamps, and
  could disagree with them.

## R5. Admin view of API tokens

- **Decision**:
  - **List**: A new `Cmd::ListAllApiTokens { holder, state }` returns `Event::AllApiTokens`. It is
    for instance admins only, and every row carries the holder's display name.
  - **Revoke**: `PreviewApiTokenRevoke` is reused. When the caller isn't the token's owner, the
    preview requires step-up (`step_up: true`) and the audit context records `by_admin: true`.
    Revoking your own token is unchanged.
- **Rationale**: One revoke path. FR-021's passkey rule applies to the admin action, without
  making a person's own revoke stricter than it is today.

## R6. The Hosts workspace picker (FR-002)

- **Decision**: The picker reads `ListWorkspaces`, which already returns every workspace to
  instance admins, instead of `me.grants`. The Enroll button is shown when the caller holds
  `operate` on at least one listed workspace.
- **Rationale**: The server already has the right answer. The bug is only in the web picker.

## R7. Instance roles from OIDC claims

- **Decision**:
  - **Configuration** (FR-024, FR-030), as environment variables with matching flags:
    - `KEEL_OIDC_ROLE_CLAIM`: a dotted path into the ID token, for example `groups` or
      `realm_access.roles`. When it is unset there is no mapping.
    - `KEEL_OIDC_OWNER_VALUES`, `KEEL_OIDC_ADMIN_VALUES` and `KEEL_OIDC_USER_VALUES`:
      comma-separated lists.
    - `KEEL_OIDC_ROLE_REQUIRED`: a boolean, default `false`.
  - **Startup refusals**: Keel refuses to start when:
    - any value list is set without the claim;
    - the claim is set with every value list empty;
    - the claim is set without OIDC configured.
  - **Reading the claim**: The claim is read through `web_auth`'s groups claim. `ClaimsConfig.groups`
    learns dotted paths: a path segment walks into objects, and the leaf must be a string array.
    Fathom's existing single-segment configs behave exactly as before.
  - **Prefix**: Keel sets `groups_prefix` to empty.
  - **Missing claim**: `groups_required` is set to `KEEL_OIDC_ROLE_REQUIRED`. A missing claim is
    therefore a sign-in error only when a match is required, which matches the edge case "missing
    claim never means no change".
- **Sign-in** (`identity::sign_in`, already under the `SIGN_IN_LOCK` advisory lock):
  - **Mapping active and the provider is `oidc`**: `mapped = highest(owner, admin, user)` over the
    identity's groups.
    - With `None` and the mapping required: refuse with `SignInError::NotMapped`. No row is
      written. The refusal is audited through `signin::refused` with reason `role_not_mapped`.
    - With `None` and not required: the role is user.
    - The role is applied on create and on every sign-in. A changed role:
      - updates `instance_role`;
      - sets `role_source = 'idp'`;
      - calls `bump_epoch`, which ends sessions issued under the old role (FR-027);
      - is audited as `user_role` with reason `identity_provider`.
    - **Last-owner guard (FR-028)**: if the change would demote the last active owner, the role
      stays owner and `idp_role` stores the mapped role. The audit entry has reason
      `idp_names_no_owner`.
  - **Mapping active and the provider is not OIDC**:
    - required: refused;
    - not required: new accounts are users, and existing accounts keep their role.
  - **No mapping**: today's rule. FR-023's race is already closed by the advisory lock: the
    `count(*)` runs inside the lock. A test pins this.
- **Users page (FR-029)**: rows with `role_source = 'idp'` are read-only, and `PreviewUserRole` is
  refused for them on the server too.
- **System Settings**: `GetSystemSettings` returns the mapping, read-only, and the warning "the
  identity provider names no owner" while no active user has `idp_role = 'owner'`.
- **Alternatives**:
  - A role mapping table edited in the UI: rejected by the spec (deployment configuration only).
  - A Keycloak admin API sync: a new dependency and a credential.

## R8. Owner-only System workspace

- **Decision**:
  - **Marking it**: `workspaces.system boolean not null default false`, with a unique partial index
    `where system`. Its slug is `keel-system`, which workspace create now refuses.
  - **Enforcement in one place**: `keel_core::authz::Subject` gains
    `system_workspace: Option<WorkspaceId>`.
    - `require` refuses any subject that isn't an owner on that workspace, with the new refusal
      `owner_only`, before the admin bypass.
    - `Caller` fills it from `AppState.system_workspace`, a `RwLock<Option<Uuid>>` loaded at start
      and updated on configure and remove.
  - **Lists that aren't workspace-scoped filter the System workspace out for non-owners**:
    `ListWorkspaces`, the audit list, `ListAllApiTokens` and the Tokens tab enrolment list.
  - **Grants**: `PreviewGrantAdd` and `PreviewApiTokenCreate` are refused on the System workspace.
    Only the instance role grants access.
  - **Configuration**: a single-row `system_config` table holding `workspace_id`,
    `allowed_profiles text[]` and `version`.
    - `PreviewSystemConfigure { allowed_profiles }` (owner, step-up) creates the workspace, or
      reuses and un-archives it, and writes the configuration.
    - The repository binding is made with the existing `PreviewBindingCreate`, targeted at the
      System workspace. The System tab embeds the existing binding form, so FR-034's "through the
      existing binding mechanism" holds literally.
  - **Removal**: `PreviewSystemRemove` (owner, step-up), in one transaction:
    - revokes every host in the System workspace (the existing revoke path);
    - disables its bindings;
    - archives the workspace;
    - deletes the `system_config` row;
    - audits it.
- **Rationale**: A single check point in `authz::require` covers every workspace-scoped command,
  including runs, hosts, reviews and inventories, without touching each handler.
- **Alternatives**:
  - Filtering in each handler: easy to miss one.
  - A separate "system" instance: far more machinery.

## R9. Runs that restart Keel (FR-037)

- **Decision**:
  - **Normal path**: already covered by 058. The `keel_server` role defers `docker compose up` with
    `systemd-run --on-active=30s` after the play ends, so the runner sends `Finished` before Keel
    stops.
  - **Abnormal path**: Keel stops before `Finished` reaches it.
    - Runner: it writes an undelivered `Finished` frame to `<state>/undelivered/<run_id>.json`
      (mode 0600), and replays it after the next `Hello` is accepted.
    - Server: it accepts a late `Finished` for a `run_hosts` row that is `lost` with the reason
      `runner_lost`, records the real outcome, and audits `run_result_late`.
    - Replayed frames older than `KEEL_RUNNER_MAX_OFFLINE` are dropped.
- **Rationale**: The "Keel didn't come back in time" case shows the true outcome once Keel is up,
  with no polling.
- **Alternatives**: Report the result through the outside-run channel (A2). Rejected: those runs
  are Keel's own, not outside ones.

## R10. The polish pass (US6)

- **Decision**:
  - **Shared primitives**: `apps/keel/web/src/ui/` gets `Dialog`, `CopyBlock`, `EmptyState`,
    `LoadState`, `Tabs`, `DataTable` and `PageHeader`, built on the existing `radix-ui` dependency
    and the theme tokens in `index.css`.
  - **Migration**: every section moves to them.
  - **Review**: the `impeccable` skill runs over the result as the design review: critique, polish,
    then harden.
  - **Validation**: Playwright screenshots of each section at 360 px and 1280 px, in both themes.
- **Rationale**: FR-031 is about one shared set of treatments. Shared components are how that stays
  true after the pass.

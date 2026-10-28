# Contract: the System workspace (spec 060, User Story 7)

## Lifecycle

| Step | Command | Effect |
|---|---|---|
| Configure | `PreviewSystemConfigure { allowed_profiles, version }` | Creates the workspace `keel-system` (`system = true`), or un-archives it. Writes `system_config`. Audits `system_configure`. |
| Bind the repo | existing `PreviewBindingCreate` on the System workspace | As for any workspace. The System tab embeds the existing binding form. |
| Enrol the Keel host | existing Enroll dialog, System workspace picked | Profiles are limited to `allowed_profiles`; others are refused with `profile_not_allowed`. |
| Run | existing run commands | Owner only (see below). |
| Remove | `PreviewSystemRemove` | One transaction: revokes the hosts, disables the bindings, archives the workspace, deletes `system_config`, audits `system_remove`. |

`version` follows the same optimistic rule as settings: a stale version is refused with
`system_conflict`.

## Enforcement

- **`authz::require`**: when the target workspace equals `Subject.system_workspace` and the
  subject's instance role isn't owner, it returns `Refusal::OwnerOnly` (`code() = "owner_only"`).
  This check runs before the instance-admin bypass.
- **API tokens**: refused on the System workspace, at creation and at use.
- **Lists that cross workspaces**: the System workspace and its rows are left out for non-owners.
  These are `ListWorkspaces`, the audit list, `ListAllApiTokens` and `ListEnrollments`.
- **Not configured**: `GetSystem` returns `configured: false`, and the nav shows no System entry
  (FR-039).

## UI

- **Where**: System Settings gains a "System" tab, for owners only.
- **Not configured**: an empty state that explains the feature, and a "Set up" action.
- **Configured**:
  - the allowed profiles, editable;
  - the binding, with its repo, branch and state;
  - the enrolled Keel host, if any, with a link to Enroll;
  - recent runs, linking to the existing run view;
  - "Remove", behind a typed confirmation of `keel-system`.

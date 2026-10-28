# Instance roles from your identity provider

Keel can take each person's instance role (owner, admin or user) from your OpenID Connect
identity provider. You manage people in one place, such as a Keycloak realm, and Keel follows.

- Members of the groups or roles you name for owners sign in as owners.
- Admins and users work the same way, each with their own list of names.
- If you choose, anyone who matches none of the names is turned away at sign-in.

Without a mapping, Keel keeps its built-in rule. The first person to sign in to an empty instance
becomes owner, and everyone after that becomes a user. This holds even when two people sign in at
the same moment.

## How roles are decided

- **Every sign-in counts.** Keel reads the claim each time someone signs in through your identity
  provider, not only the first time.
- **The highest match wins.** Someone in both `keel-admins` and `keel-users` becomes an admin.
- **A changed role takes effect at the next sign-in.** When you move someone to another group, their
  role changes the next time they sign in. Keel records the change in the audit log with the reason
  "identity provider", and ends the sessions they opened under their old role.
- **Keel always keeps an owner.** If a sign-in would demote the last owner, that person stays owner
  and the sign-in goes ahead. The audit log records that the identity provider names no owner, and
  System Settings shows a warning until someone your provider names as owner signs in.
- **Roles are read-only in Keel.** On the Users page, people whose role comes from the identity
  provider are tagged "Identity provider", and their role can't be changed there. Change their
  groups in your identity provider instead.

## Settings

Set these alongside your other `KEEL_OIDC_*` settings. Each one also has a command-line flag, and
the flag wins when both are set.

| Variable | Flag | What it does |
|---|---|---|
| `KEEL_OIDC_ROLE_CLAIM` | `--oidc-role-claim` | The claim holding the groups or roles, such as `groups` or `realm_access.roles`. Dots step into nested claims. Leave unset for no mapping. |
| `KEEL_OIDC_OWNER_VALUES` | `--oidc-owner-values` | Comma-separated names that grant owner |
| `KEEL_OIDC_ADMIN_VALUES` | `--oidc-admin-values` | Comma-separated names that grant admin |
| `KEEL_OIDC_USER_VALUES` | `--oidc-user-values` | Comma-separated names that grant user |
| `KEEL_OIDC_ROLE_REQUIRED` | `--oidc-role-required` | `true` or `false` (default `false`). See below. |

### When a match is required

With `KEEL_OIDC_ROLE_REQUIRED=true`:

- Someone who matches none of the names is refused, and is told to ask an administrator.
- No account is created for them, and the refusal is recorded in the audit log.
- People signing in through another provider, such as GitHub, are refused too.

With `false`, the default:

- Someone who matches none of the names signs in as a user.
- People signing in through another provider get new accounts as users, and existing accounts keep
  their role.

### Settings Keel refuses

Keel won't start with a mapping it can't apply, and the error names the setting. This happens when:

- a value list or `KEEL_OIDC_ROLE_REQUIRED` is set without `KEEL_OIDC_ROLE_CLAIM`;
- `KEEL_OIDC_ROLE_CLAIM` is set but OIDC isn't configured;
- `KEEL_OIDC_ROLE_CLAIM` is set but all three value lists are empty;
- the claim has an empty part, such as `realm_access..roles`;
- a list has an empty entry, such as `a,,b`.

## Example: Keycloak realm roles

1. In your realm, create the roles `keel-owners`, `keel-admins` and `keel-users`, and assign them to
   people.
2. Make sure the roles reach Keel's ID token. Keycloak sends realm roles as `realm_access.roles`.
   Open **Client scopes**, then **roles**, then **Mappers**, then **realm roles**, and turn on
   **Add to ID token**. Alternatively, add a "User Realm Role" mapper to the `keel` client, with
   the token claim name `realm_access.roles`, added to the ID token.
3. Configure Keel:

   ```env
   KEEL_OIDC_ROLE_CLAIM=realm_access.roles
   KEEL_OIDC_OWNER_VALUES=keel-owners
   KEEL_OIDC_ADMIN_VALUES=keel-admins
   KEEL_OIDC_USER_VALUES=keel-users
   KEEL_OIDC_ROLE_REQUIRED=true
   ```

4. Restart Keel, then sign in as someone in `keel-owners`.

To use Keycloak groups instead of roles, add a "Group Membership" mapper to the `keel` client:

- set the token claim name to `groups`;
- turn off **Full group path**;
- turn on **Add to ID token**.

Then set `KEEL_OIDC_ROLE_CLAIM=groups` and list the group names.

## What owners and admins see

**System Settings** shows the active mapping, read-only:

- the provider;
- the claim Keel reads;
- the names for each role;
- whether a match is required.

To change the mapping, change Keel's deployment settings and restart. It can't be edited in the
browser, so nobody can use it to make themselves owner.

If no active person is named as owner by your identity provider, System Settings warns that the
identity provider names no owner. Add someone to your owner group and have them sign in.

## Turning the mapping off

Remove `KEEL_OIDC_ROLE_CLAIM` and the value lists, then restart Keel.

- People keep the role they last had.
- Their roles stay tagged "Identity provider" until an admin changes them on the Users page.
- From then on, Keel manages those roles again.

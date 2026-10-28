# Contract: role-mapping configuration (spec 060, User Story 5)

These settings sit alongside the existing OIDC ones in `apps/keel/server/src/config.rs`. Every
variable also has a `--oidc-…` flag, and the flag takes precedence over the variable, as the other
settings do.

| Variable | Flag | Meaning |
|---|---|---|
| `KEEL_OIDC_ROLE_CLAIM` | `--oidc-role-claim` | Dotted path to a string-array claim, for example `groups` or `realm_access.roles`. Unset means no mapping. |
| `KEEL_OIDC_OWNER_VALUES` | `--oidc-owner-values` | Comma-separated claim values that grant owner |
| `KEEL_OIDC_ADMIN_VALUES` | `--oidc-admin-values` | … that grant admin |
| `KEEL_OIDC_USER_VALUES` | `--oidc-user-values` | … that grant user |
| `KEEL_OIDC_ROLE_REQUIRED` | `--oidc-role-required` | `true` or `false` (default `false`). When true, people who match no value, and people signing in through another provider, are refused. |

Keel refuses to start (FR-030), naming the setting, when:

- any `*_VALUES` or `ROLE_REQUIRED` is set without `ROLE_CLAIM`;
- `ROLE_CLAIM` is set but OIDC isn't configured (no issuer or client);
- `ROLE_CLAIM` is set and all three value lists are empty;
- `ROLE_CLAIM` has an empty segment (`a..b`, a leading or trailing `.`);
- a value is empty after trimming (`a,,b`).

## Keycloak example (docs/guides)

1. On the `keel` client, add a mapper "User Realm Role", with token claim name `realm_access.roles`,
   added to the ID token. Or use the default `realm_access` claim, which Keycloak emits already.
2. Set:

   ```env
   KEEL_OIDC_ROLE_CLAIM=realm_access.roles
   KEEL_OIDC_OWNER_VALUES=keel-owners
   KEEL_OIDC_ADMIN_VALUES=keel-admins
   KEEL_OIDC_USER_VALUES=keel-users
   KEEL_OIDC_ROLE_REQUIRED=true
   ```

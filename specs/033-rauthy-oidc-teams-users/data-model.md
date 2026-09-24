# Data Model: 033-rauthy-oidc-teams-users

## Persisted (additive, `~/.fathom/config.json` key `oidc`; unreadable -> defaults, file never deleted)

| Entity | Fields |
|---|---|
| OidcIssuerConfig | `id` (slug), `issuer_url`, `client_id`, `scopes[]` (default openid profile email groups), `ca_bundle_path?`, `username_claim` (default preferred_username), `groups_claim` (default groups), `username_prefix?`, `groups_prefix?`, `admin_api_base?` |
| ContextAuthMode | `context_name`, `mode: kubeconfig \| oidc` (default kubeconfig), `issuer_id?` |
| FathomRoleMap | `viewers_group`, `editors_group`, `admins_group` (names) |

## Credential store (OS keyring, never `~/.fathom/`)

`fathom.oidc.<issuer_id>`: `refresh_token`, optional cached `id_token`. `fathom.rauthy.<issuer_id>`: admin API key.

## Team store (014, additive)

`TeamLink { group, profile_id }` list; older readers ignore the field.

## Runtime / protocol types (Rust is the source; ts-rs generated)

- `OidcIdentity { issuer_id, sub, username, email?, groups[], expires_at, state: SignedIn | Refreshing | SignedOut{reason} }`
- `IdpUser { sub, username, email, groups[], enabled }`, `IdpGroup { name, member_count }`, `IdpDirectorySnapshot { users, groups, fetched_at, stale }`
- `CapabilityQuery { context, namespace?, verb, resource, subresource? }`, `CapabilityResult { allowed, reason?, source: Rules | Review, verified }`
- `CapabilitySet { context, namespace, identity_sub, rules[], incomplete, fetched_at }`
- `RoleAssignmentDraft { subject, role_ref, namespace?, rendered_subject_name }` (rendering only; the write is `ApplyResourceYaml`)
- `OrphanedSubject { binding, subject_name }`

## Rules and transitions

- Identity keyed by (issuer_id, sub). States: SignedOut -> SignedIn -> Refreshing -> SignedIn | SignedOut{reason}.
- Capability cache entry invalid on identity change, context change, TTL 60 s, or successful RBAC apply.
- Rendered subject name = `groups_prefix + group` (or `username_prefix + username`). If the operator has not set a prefix/claim mapping, the preview warns instead of guessing.

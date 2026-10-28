# Quickstart: validating spec 060

## Prerequisites

- A running dev stack: the Keel server against the throwaway test database (`eval "$(scripts/keel-test-db.sh start)"`; `make keel test` does this for you) and the web dev server.
- An owner account, an admin account and a user account. Optionally, the Keycloak dev realm with
  the `keel-owners`, `keel-admins` and `keel-users` roles.

Detailed shapes are in [contracts/](contracts/) and [data-model.md](data-model.md).

## Targeted tests

Run these one at a time, with `CARGO_BUILD_JOBS=4`:

```sh
cargo test -p keel-core enroll_template      # vectors from contracts/template-vectors.json
cargo test -p keel-core authz                # owner_only refusal
cargo test -p web-auth                       # dotted claim paths
cargo test -p keel-server                    # settings, tokens, roles, system, late results
cargo test -p keel-runner                    # undelivered spool
pnpm --filter @companion/keel-web test                  # template twin, Markdown, tabs
pnpm --filter @companion/keel-web exec playwright test tests/e2e/enroll.spec.ts
```

## Scenarios

| # | Do | Expect |
|---|---|---|
| 1 | Sign in as an owner with no team grants. Open Hosts. | The Enroll button shows, and the picker lists every workspace (US1, FR-002). |
| 2 | Enroll `web-01` with the base profile. | The token and snippet show once. The snippet matches the default template, with `%t` replaced. Closing warns. |
| 3 | System Settings, Enrollment: type `%x`. | Inline `unknown_placeholder`; Save is disabled. |
| 4 | Save a template plus Markdown with `<script>`. Enroll again. | The new snippet is used. `<script>` shows as text. An audit row `settings_update` is written. |
| 5 | Save from two tabs. | The second gets the conflict banner (FR-014). |
| 6 | Tokens: Revoke a pending token, then run `make keel-node` with it. | It is refused exactly like a burned token. |
| 7 | Tokens: Roll. | A new token shows once. The old one shows as revoked, and the new one links to it with `rolled_from`. |
| 8 | Revoke another user's API token. | Step-up is asked, and the audit entry has `by_admin: true`. |
| 9 | Set the role mapping. Sign in as someone in `keel-admins`. | They are an admin, and their row is read-only on Users. |
| 10 | Remove that person from the group, and sign in again. | They are a user, and their old sessions end. |
| 11 | Set `ROLE_REQUIRED=true`. Sign in with no mapped group. | Refused; no account is created. |
| 12 | Owner: System tab, Set up. Bind the ansible repo. Enroll the Keel host. | The workspace exists. Admins don't see it in Hosts, Audit or Tokens. |
| 13 | As an admin, call `ListHosts` on the System workspace. | `Denied { owner_only }`. |
| 14 | Run `keel-server` from the System workspace. | The run finishes. Keel restarts afterwards (058's deferred up), and the result shows. |
| 15 | Kill the server mid-run, then start it again. | The run shows `runner_lost`, then the real outcome, with an audit `run_result_late`. |
| 16 | Polish: run `polish-screens.spec.ts`. | Screens at 360 and 1280 px, in both themes, show no overflow and no horizontal scroll. |

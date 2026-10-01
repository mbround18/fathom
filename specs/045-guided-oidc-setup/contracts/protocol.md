# Protocol contract: 045-guided-oidc-setup

Additive only: new command and event variants, no changed or removed ones, so the protocol stays
**v1** (the 034 precedent; `constitution.md:41` requires a bump only for a change that breaks
existing clients). Every variant below needs a sample in `tests/protocol_serde.rs` and an arm in the
matching `*_is_exhaustive` match, and a classification in all four of `mutation.rs`'s wildcard-free
matches (`mutation_target` `:46`, `section` `:413`, `audit_summary` `:723`, `hosted_policy` `:1348`).

## Hosted, narrower than FR-031 reads

FR-031 says hosted gets the guided path's steps 1-7. In the code that cannot mean the draft
commands run hosted: `mutation.rs` already denies *every* OIDC configuration command as
`SharedState`, on the stated grounds that anything touching server-shared state or server-held
credentials is refused, and `config.json` is exactly that shared state. So hosted gets:

- `ReadApiServerAuth` (allowed -- the signed-in user's own RBAC governs it, FR-023), and
- a server-emitted `OidcSeedOrigin` carrying the issuer the server validated at startup (FR-031),
  rendered read-only and reviewable (FR-032).

The three draft commands and `WriteApiServerAuth` are denied. The seeded issuer reaches the UI
through the new event, not by unblocking `LoadOidcConfig`.

## New commands

### `OidcDiscoverDraft`

```json
{ "type": "OidcDiscoverDraft", "data": { "issuer_url": "https://idp.example.com/realms/fathom" } }
```

- Mutation class: `ReadOnly`. `section()` returns `None` (identity is global, as for every other
  OIDC command). Not audited.
- Needs no saved issuer and no connected cluster. Reply: one `OidcDraftDiscovered`.
- **Denied in hosted** (`ServerSideProbe`): it would make the server fetch a URL the caller chose,
  the same reason `TestPrometheusConnection` is refused. Hosted does not need it -- the server
  already discovered its own issuer before it bound a socket.
- A failed probe is `CommandFinished { ok: false, error }` with the provider's verbatim message.

### `OidcTestSignIn`

```json
{ "type": "OidcTestSignIn", "data": { "draft": { "config": { "id": "idp", "issuer_url": "https://idp.example.com/realms/fathom", "client_id": "fathom", "scopes": ["openid","profile","email","groups"], "username_claim": "preferred_username", "groups_claim": "groups" }, "method": "Browser" } } }
```

- Mutation class: `ReadOnly` (it writes no cluster and, deliberately, no local credential).
  `section()` returns `None`. Not audited.
- **Denied in hosted** (`SharedState`): it drives the process-wide session machinery.
- Carries the whole draft inline because `OidcSignIn { issuer_id, method }` resolves the issuer out
  of *saved* config (`cmd/oidc.rs:91`) and `store.rs` keys credentials on `fathom.oidc.<issuer_id>`.
- **Writes nothing**: the tokens stay in the in-memory `Live` slot and never reach the keyring.
  Reply: the existing sign-in progress events, then one `OidcDraftSignInSucceeded`.

### `OidcSaveDraft`

```json
{ "type": "OidcSaveDraft", "data": { "draft": { "config": { "…": "…" }, "method": "Browser" }, "role_map": { "viewers_group": "fathom-viewers", "editors_group": "fathom-editors", "admins_group": "fathom-admins" }, "adopt_test_session": true } }
```

- Mutation class: `Local`. `section()` returns `None`. **Not audited** -- it writes no cluster
  object, exactly like the `OidcSaveIssuer` it generalises, and FR-024 requires an audit entry for
  *cluster* writes. The protocol module's own `OidcSaveIssuer => None` arm is the precedent.
- **Denied in hosted** (`SharedState`): `config.json` is the state directory shared by every user
  of the process, so one user's issuer edit would re-point everyone else's sign-in. Hosted's issuer
  comes from the server's validated startup configuration instead (FR-031, FR-032).
- The single transactional save: persists the issuer and the role map, and with
  `adopt_test_session: true` commits the test sign-in's tokens under the now-saved id so the
  operator is signed in without a second sign-in (FR-028). `false` saves without a session.
- Refused if `adopt_test_session` is set and no test sign-in is live.

### `ReadApiServerAuth`

```json
{ "type": "ReadApiServerAuth" }
```

- Mutation class: `ReadOnly`. `section()` returns `None`. Not audited.
- An ordinary namespaced read of the `kube-apiserver` static pod in `kube-system`, performed as the
  signed-in user. Requires a connected cluster. **Allowed in hosted.**
- Reply: one `ApiServerAuthRead` event. A denied or impossible read is a *successful* command with
  `checked: false` and the API server's verbatim `reason` — never an error and never an assumed pass.

### `WriteApiServerAuth`

```json
{ "type": "WriteApiServerAuth", "data": { "cluster": "fathom-dev", "style": "Structured", "issuer_url": "https://idp.example.com/realms/fathom", "client_id": "fathom", "username_claim": "preferred_username", "groups_claim": "groups", "username_prefix": "oidc:", "groups_prefix": "oidc:" } }
```

- Mutation class: `ClusterScoped`, so `writes_cluster()` is true and the devbridge guard applies.
  `section()` returns `None`. **Audited**, under a new `AuditActionKind::ApiServerAuthChange`: no
  existing variant fits, and FR-024 requires the one cluster write of this feature be recorded.
- **Denied in hosted** (`Kind`): must *also* be added by name to `server/src/filter.rs`'s `DENIED`
  const, because `table()` defaults an unrecognised variant to **allowed** — an omission silently
  permits it. Rationale: writing the flags means editing a manifest on a node, which only ever
  applies to a local cluster Fathom provisioned; a hosted server has no node to write to.
- Refused unless the named cluster carries the provenance record of FR-036. Writes in the style the
  cluster already uses, never both (`Legacy` and `Structured` are mutually exclusive).
- Runs inline in its handler: `audit_summary()` is taken at dispatch (`src/backend.rs:312`), so an
  asynchronous completion would be audited before it happened.

## New events

### `OidcDraftDiscovered` (`SystemEvent`)

```json
{ "type": "OidcDraftDiscovered", "data": { "issuer_url": "https://idp.example.com/realms/fathom", "discovery": { "authorization_endpoint": "https://idp.example.com/realms/fathom/protocol/openid-connect/auth", "token_endpoint": "…", "device_authorization_endpoint": "…", "jwks_uri": "…", "scopes_supported": ["openid","profile","email","groups"], "claims_supported": ["sub","preferred_username","groups","email"] } } }
```

`scopes_supported` and `claims_supported` are new on `Discovery` — the private `Doc` at
`flow.rs:133` parses six fields and drops exactly these two, which FR-001 renders.

### `OidcDraftSignInSucceeded` (`SystemEvent`)

```json
{ "type": "OidcDraftSignInSucceeded", "data": { "username": "alice", "groups": ["platform","oncall"], "claims": [{ "name": "preferred_username", "value": "alice" }], "expires_at": 1790000000 } }
```

No token material crosses the protocol — only what the sign-in proved.

### `ApiServerAuthRead` (`ResourceReadEvent`)

```json
{ "type": "ApiServerAuthRead", "data": { "read": { "style": "Legacy", "issuer": "https://old.example.com/realms/k8s", "client_id": "kubernetes", "username_claim": "sub", "groups_claim": "groups", "username_prefix": null, "groups_prefix": null, "context": "kind-fathom-dev", "checked": true, "reason": null }, "differences": [{ "field": "issuer", "cluster": "https://old.example.com/realms/k8s", "draft": "https://idp.example.com/realms/fathom" }], "writable": true, "not_writable_reason": null } }
```

A stated unknown is the same event with `checked: false`, `style: "Unknown"` and `reason` carrying
the API server's words verbatim.

### `OidcSeedOrigin` (`SystemEvent`)

```json
{ "type": "OidcSeedOrigin", "data": { "issuer_id": "hosted", "origin": "SeededFromEnvironment", "source": "FATHOM_OIDC_* (mounted Secret)" } }
```

Emitted by the hosted server so the UI can render the seeded issuer read-only and say where it is
changed. Desktop never emits it, and `Saved` is the absence of it.

## Reused commands (unchanged)

| Action | Command | Notes |
| --- | --- | --- |
| Sign in with a saved issuer | `OidcSignIn { issuer_id, method }` | the saved path is untouched; the draft path bypasses `find_issuer` |
| Set the Fathom role map alone | `OidcSetRoleMap` | still available outside the path |
| Preview a cluster binding | `PreviewRoleAssignment` | 042; reached after the save, with the configured issuer's id (FR-030) |
| Apply a binding | `ApplyResourceYaml` | 042; unchanged, including its confirm step |
| Discover API groups for the role form | `RefreshApiResources` | 042; unchanged |

## Devbridge guard

`WriteApiServerAuth` writes the cluster, so `MutationTarget::writes_cluster()` is true and
`tools/devbridge/src/guard.rs`'s `check_strict` (`:81`) applies: in E2E it is permitted only against
the fenced test clusters. `ReadApiServerAuth` is read-only and needs no guard entry.

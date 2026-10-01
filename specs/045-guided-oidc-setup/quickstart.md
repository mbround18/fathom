# Quickstart: guided OIDC setup

## Try it

1. Start the desktop app against the dev cluster: `cargo tauri dev` (or `pnpm -C ui dev` with the
   dev bridge). Open the identity chip in the top bar.
2. Click **Add issuer…**. Paste only an issuer URL — e.g. the lab Keycloak at
   `https://idp.127.0.0.1.nip.io/realms/fathom` — and continue. The path shows the discovery
   document it fetched, including the scopes and claims the provider advertises.
3. Continue to the claims step. The username and groups claims are pre-filled from what the provider
   says it supports; change either and the path keeps your choice.
4. Run the **test sign-in**. The browser flow completes and the path renders the decoded claims and
   the groups the token actually carried. Nothing has been saved yet — close the Sheet here and
   confirm the issuer list is still as it was, with no credential in the keyring.
5. Reopen the path and reach the **cluster comparison**. It names the context it read and either
   shows a row per difference between the cluster's configuration and your draft, or states that it
   could not read the API server, verbatim, without blocking.
6. Fill the **Fathom role slots** from the groups the sign-in returned. The step says plainly that
   these gate Fathom's own features and grant nothing in the cluster.
7. **Save**. You are signed in from the test sign-in — no second sign-in — and the issuer now appears
   in the list.
8. After the save, take the **cluster bindings** handoff into 042's role form, and confirm the
   issuer it carries is the one you just configured, not the first issuer in the list.
9. On a cluster Fathom provisioned, the **API server write** is offered, showing the backup it takes
   and the exact flags it will set. On any other cluster — including every hosted deployment — the
   same step is copyable text instead, saying Fathom will not change a control plane it does not own.
10. Hosted: run the server with `FATHOM_OIDC_ISSUER`/`FATHOM_OIDC_CLIENT_ID` set from a mounted
    Secret (`pnpm -C ui e2e:hosted` fixtures, or `cargo make hosted-e2e`). The issuer list shows the
    seeded issuer, labelled with its origin, read-only, and still fully reviewable through steps
    1–7. As a user the cluster denies, the configuration surface is refused with the API server's
    own reason.

## Tests

```bash
# protocol: round-trips for every new variant, plus the exhaustive matches
cargo test --all

# bindings must not drift (Principle III); regenerates ui/src/protocol/generated/
cargo test -p fathom --test export_bindings

# the pure step machine and the store reducers
cd ui && pnpm test

# the step sequence, the save boundary and the stated-unknown read, against the fake transport
cd ui && pnpm e2e --grep "guided oidc"

# hosted: the seeded issuer renders read-only and the kind write is denied
cd ui && pnpm e2e:hosted

# everything the CI gate runs
scripts/ci.sh all
```

Run `pnpm` from `ui/`, not the repo root (the root pin and the global version differ). Use
`CARGO_BUILD_JOBS=4` when other agents are active, and `scripts/hygiene.sh` before reporting done.

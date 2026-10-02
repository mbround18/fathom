# Quickstart: guided OIDC setup

Corrected against what shipped by T076, which ran it end to end: steps 1–8 on the dev cluster
(`kind-fathom-dev`), step 9 on a throwaway Fathom-provisioned cluster, because the dev cluster is
deliberately not writable — see the note under step 9.

## Try it

1. Start the desktop app against the dev cluster: `pnpm -C ui tauri dev` (or `pnpm -C ui dev` with
   the dev bridge). Open the identity chip in the top bar, then **Settings → Identity**.
2. Click **Add issuer with the guided path**. Paste only an issuer URL — e.g. the lab Keycloak at
   `https://idp.127.0.0.1.nip.io/realms/fathom` — and leave the field. *What the provider
   advertises* shows the discovery document it fetched, including the scopes and claims the
   provider declares. A declared `issuer` that disagrees with the URL you typed blocks progress and
   shows both.
3. Run the **Test sign-in**. The browser flow completes and the path renders the decoded claims and
   the groups the token actually carried. Nothing has been saved yet — close the Sheet here and
   confirm the issuer list is still as it was, with no credential in the keyring.

   The sign-in comes *before* the claims step, not after: FR-005 wants the claims chosen against a
   real token rather than against the provider's advertisement. Without a sign-in the claims step
   still works, from the advertised claims, and says that is what they are.
4. Continue to **Claims and prefixes**. The username and groups claims are pre-filled from the
   token the sign-in returned; change either and the path keeps your choice, with a preview of the
   prefixed result.
5. Reach **What this cluster accepts**. It names the context it read and either shows a row per
   difference between the cluster's configuration and your draft, or states that it could not read
   the API server, verbatim, without blocking. On the dev cluster, whose API server carries no OIDC
   flags at all, every field reads as absent rather than as agreeing.
6. Fill the **Fathom role groups** from the groups the sign-in returned. The step says plainly that
   these gate Fathom's own features and grant nothing in the cluster.
7. **Review and save**. You are signed in from the test sign-in — no second sign-in — and the issuer
   now appears in the list.
8. After the save, take the **Cluster bindings** handoff into 042's role form, and confirm the
   issuer it carries is the one you just configured, not the first issuer in the list.
9. **API server configuration.** On a cluster Fathom provisioned, the write is offered, showing the
   backup it takes and the exact flags it will set. On any other cluster — including every hosted
   deployment — the same step is copyable text instead, saying Fathom will not change a control
   plane it does not own.

   Ownership is read from `~/.fathom/kind-provenance.json`, never inferred from the `kind-` context
   name, so **the dev cluster is not writable**: it was created outside Fathom and has no record.
   That is the refusal half of this step, and it is what you will see on `kind-fathom-dev`. To
   exercise the write itself, provision a cluster through Fathom's own kind path — and delete it
   afterwards (CLAUDE.md).
10. Hosted: bring the server up with `k8s/cluster.sh up-hosted`, which sets
    `FATHOM_OIDC_ISSUER`/`FATHOM_OIDC_CLIENT_ID` from a mounted Secret. The issuer list shows the
    seeded issuer, labelled with its origin, read-only, and still fully reviewable through steps
    1–8. As a user the cluster denies, the configuration surface is refused with the API server's
    own reason.

## Tests

```bash
# protocol: round-trips for every new variant, plus the exhaustive matches
cargo test --all

# bindings must not drift (Principle III); regenerates ui/src/protocol/generated/
cargo test -p fathom --test export_bindings

# the pure step machine and the store reducers
pnpm -C ui test

# the step sequence, the save boundary and the stated-unknown read, against the fake transport.
# The titles start with "045 ", so grep for that; `--grep "guided oidc"` matches nothing.
pnpm -C ui e2e -- --grep "^045"

# hosted: the seeded issuer renders read-only and the kind write is denied.
# This spec lives in tests/e2e, so `e2e:hosted` (testDir tests/hosted) does NOT run it.
pnpm -C ui e2e -- hosted-guided-oidc

# everything the CI gate runs
scripts/ci.sh all
```

### Against a live cluster

Both are `#[ignore]`d, per the repo's convention for tests that need a cluster. The read is safe
anywhere; **the write restarts the control plane twice**, so give it a throwaway cluster and delete
it afterwards.

```bash
# reads the API server's own flags, then checks the write is refused on an unowned cluster
FATHOM_APISERVER_LIVE_CONTEXT=kind-fathom-dev \
  cargo test -p fathom --lib live_api_server_read -- --ignored --nocapture

# the rollback, the real readiness wait, the read-back through the Kubernetes API, and the restore
kind create cluster --name fathom-prov-045
FATHOM_APISERVER_LIVE_CLUSTER=fathom-prov-045 \
  cargo test -p fathom --lib live_api_server_write -- --ignored --nocapture
kind delete cluster --name fathom-prov-045
```

These found four defects that no fake node could have: a `docker exec` with no stdin that truncated
the manifest and reported success, an unquoted `oidc:` prefix that made the kubelet reject the pod
outright, and an in-place write — on both the write and the rollback path — that let the kubelet
read the manifest half-written and leave its mirror pod on the superseded spec. See `1f2ce78`.

Run `pnpm` from `ui/`, not the repo root (the root pin and the global version differ). Use
`CARGO_BUILD_JOBS=4` when other agents are active, and `scripts/hygiene.sh` before reporting done.

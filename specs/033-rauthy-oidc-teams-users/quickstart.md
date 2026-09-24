# Quickstart: 033-rauthy-oidc-teams-users

Validation runs against the existing kind Rauthy (`kind-fathom-oidc-rauthy`, issuer `https://auth.localtest.me/auth/v1/`). Dev-lab credentials only; not production.

## 0. Lab setup

1. `kubectl --context kind-fathom-oidc-rauthy -n rauthy get deploy rauthy` is Available.
2. Apply the client/key patch: `kubectl --context kind-fathom-oidc-rauthy apply -k k8s/rauthy-clients` (adds public client `fathom-desktop` and read-only key), then restart Rauthy so bootstrap reseeds (data is `emptyDir`).
3. Configure the target cluster's API server for the issuer with `k8s/kind-oidc-apiserver` (client id `fathom-desktop`, username claim `preferred_username`, groups claim `groups`, prefixes `oidc:`). Add the node hosts entry and CA file described there.
4. Bind groups: `fathom-editors` -> ClusterRole `edit` in ns `fathom-dev`; `fathom-viewers` -> `view`.

## 1. Story 1 (sign-in)

Settings -> Identity: add issuer, client id `fathom-desktop`, Sign in as alice. Expect chip `alice` + group `fathom-viewers`. Wait > 60 s: identity stays signed in (silent refresh). Sign out: chip clears. Block the loopback port: device-code flow is offered.

## 2. Story 2 (cluster login)

Mark the kind context "OIDC". Connect. Expect `SelfSubjectAccessReview` identity `oidc:alice`. Break the issuer config: verbatim API server error, no fallback to kubeconfig.

## 3. Stories 5 and 3 (scoped UI)

As alice: delete/scale/restart/apply/node shell/port-forward disabled with reason. Sign in as bob: Scale, Delete, Restart enabled in `fathom-dev`, disabled elsewhere. Carol: no-access state. With the API server stopped: controls disabled "could not verify".

## 4. Story 4 (role assignment)

Roles -> Assign: pick group `fathom-editors`, ClusterRole `edit`, ns `fathom-dev`. Preview shows subject `Group oidc:fathom-editors`. Confirm; bob's capabilities refresh. Delete the group in Rauthy: binding flagged orphaned.

## 5. Story 6 (LDAP)

`kubectl --context kind-fathom-oidc-rauthy apply -k k8s/rauthy-ldap-lab`. Register Dex as an upstream provider in Rauthy (documented steps). Sign in as the LDAP user through Rauthy: Fathom shows the mapped groups. Confirm `cargo tree | grep -i ldap` is empty.

## 6. Regression

Remove all OIDC config: kubeconfig-only usage identical to before; existing `~/.fathom/` loads unchanged.

## Checks

`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `pnpm lint && pnpm typecheck && pnpm test && pnpm e2e`, generated-binding drift check.

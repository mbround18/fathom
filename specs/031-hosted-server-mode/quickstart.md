# Quickstart: Hosted Server Mode (kind + Rauthy)

## Prerequisites

docker, kind, kubectl, helm, openssl, pnpm, Rust toolchain. Free ports 80 and 443.
Hostnames `*.localtest.me` resolve to 127.0.0.1 (public wildcard DNS); no /etc/hosts edits.

## 1. Bring up the cluster with an OIDC-aware API server and the provider

```bash
PROVIDER=rauthy k8s/cluster.sh up-hosted        # kind-oidc.yaml + ingress + Rauthy + Fathom
# Keycloak instead (separate cluster; API-server OIDC flags are fixed at create time):
PROVIDER=keycloak CLUSTER=fathom-oidc-keycloak k8s/cluster.sh up-hosted
```

The script generates a throwaway CA, builds and loads the image, seeds users, and runs
`helm install fathom deploy/helm/fathom -f k8s/overlays/hosted/values-$PROVIDER.yaml`.

## 2. Sign in as each sample user (password printed by the script)

| User | Group | Expected in namespace `demo` |
|---|---|---|
| alice | fathom-viewers | list/get everything; mutation controls hidden; a forced mutation is refused verbatim |
| bob | fathom-editors | can scale/restart/delete; other namespaces refused |
| carol | none | "No access" state within 3 s explaining what was denied |

Open https://fathom.localtest.me (accept the throwaway CA or import `k8s/.tmp/ca.crt`).

## 3. Verify audit attribution

As bob scale a Deployment in `demo`; open the Audit log: the entry names `bob`. As alice, the log
shows only alice's entries.

## 4. Verify sign-out and revocation

Sign out: live views stop and `/` redirects to the provider. Disable bob in the provider admin
UI: within 5 minutes his live views end with a sign-in prompt.

## 5. Fail-safe check

`helm upgrade fathom deploy/helm/fathom --reuse-values --set oidc.issuer=https://bad.invalid`
=> pod crash-loops with "OIDC discovery failed: ..." and `/readyz` never turns ready; no
unauthenticated listener exists.

## 6. Impersonation model

`helm upgrade ... --set accessModel=impersonation`; RBAC now binds to mapped names; audit still
names the real user.

## 7. Automated

`pnpm -C ui e2e:hosted` (PROVIDER=rauthy|keycloak) runs sign-in, renewal, sign-out, expired,
revoked, and RBAC checks. `k8s/cluster.sh down-hosted` tears everything down.

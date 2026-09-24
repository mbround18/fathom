# k8s/ — repeatable local test cluster

Kustomize manifests for a disposable kind cluster used to test Fathom against real CRDs and controllers.
Nothing here ever targets a non-kind context (`cluster.sh` pins `--context kind-$CLUSTER`).

```
k8s/cluster.sh up            # create kind cluster fathom-dev + all addons + samples
k8s/cluster.sh up minimal    # cert-manager + KEDA only
k8s/cluster.sh apply         # re-apply after editing manifests
k8s/cluster.sh down          # blow it away; `up` recreates it identically
```

- `addons/<name>` — cert-manager, keda, knative (+kourier), gatekeeper, trivy (Helm), velero (Helm + RustFS store)
- `overlays/{full,minimal}` — which addons to install
- `samples/` — workloads/CRs that populate every pane (apply after addons; CRDs must exist)

Add anything else you want to test as a new dir with a `kustomization.yaml`. Helm charts use `--enable-helm`.
Requires: docker, kind, kubectl (with kustomize), helm.

## Hosted mode (spec 031)

```
PROVIDER=rauthy   k8s/cluster.sh up-hosted   # kind cluster fathom-oidc-rauthy: OIDC API server + ingress-nginx + Rauthy + RBAC fixtures
PROVIDER=keycloak k8s/cluster.sh up-hosted   # kind cluster fathom-oidc-keycloak (API-server OIDC flags are fixed at create time)
PROVIDER=... k8s/cluster.sh down-hosted
```

- Hostnames `auth.localtest.me`, `keycloak.localtest.me`, `fathom.localtest.me` resolve to 127.0.0.1 publicly; host ports 80/443 are bound, so run one provider cluster at a time.
- `.tmp/` (git-ignored) holds a throwaway CA + leaf cert; the script mounts the CA into the API server (`--oidc-ca-file`), injects node `/etc/hosts` entries (API server) and CoreDNS `rewrite` rules (pods) pointing the names at ingress-nginx.
- Issuers: Rauthy `https://auth.localtest.me/auth/v1/` (trailing slash is part of the issuer), Keycloak `https://keycloak.localtest.me/realms/fathom`. Client `fathom` (confidential, PKCE S256, 60 s token lifetime), secret in `addons/<provider>`.
- Users `alice` (group `fathom-viewers`, view everywhere), `bob` (`fathom-editors`, edit in `demo`), `carol` (no group), all with password `FathomDemo#2026-Pw1`; IdP admin `admin@fathom.test`/`admin` with `FathomAdmin#2026-Pw1`. RBAC: `samples/hosted-rbac.yaml`. Dev-only static secrets, disposable clusters only.

## In-cluster daemon (spec 036)

```
k8s/cluster.sh up-daemon [daemon|daemon-namespaced]   # build fathom-daemon:dev (SKIP_BUILD=1 to reuse), kind load, certs, apply, samples/daemon-events.yaml
k8s/cluster.sh down-daemon                            # uninstall by label; leaves nothing behind
k8s/test-daemon-static.sh                             # offline: kustomize render, RBAC verbs, uninstall coverage (kubeconform if installed)
k8s/test-daemon.sh                                    # kind integration test (needs the real daemon image)
```

- Reached only through the aggregated API `v1.daemon.fathom.dev` (`/apis/daemon.fathom.dev/v1/...`); no Ingress, no port-forward.
- TLS trust: `addons/fathom-daemon/gen-certs.sh` makes a CA + serving cert (Secret `fathom-daemon-tls`) and patches the APIService `caBundle`; `insecureSkipTLSVerify` is only true between a bare apply and that patch. `up-daemon` re-patches after every apply.
- Everything carries `app.kubernetes.io/name=fathom-daemon`, including the cluster-scoped objects and the kube-system Role/RoleBinding. Manual uninstall: delete the `apiservice` first, then `clusterrole,clusterrolebinding,role,rolebinding,namespace -A -l app.kubernetes.io/name=fathom-daemon` (a stale APIService makes namespace deletion hang).
- `daemon-namespaced` swaps cluster-wide reads for per-namespace Roles (edit `overlays/daemon-namespaced/role.yaml` and `FATHOM_DAEMON_NAMESPACES` together).
- Image: `docker build -f deploy/Dockerfile --target fathom-daemon -t fathom-daemon:dev .`

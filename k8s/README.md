# k8s/ — repeatable local test cluster

Kustomize manifests for a disposable kind cluster used to test Fathom against real CRDs and controllers.
Nothing here ever targets a non-kind context (`cluster.sh` pins `--context kind-$CLUSTER`).

```
k8s/cluster.sh up            # create kind cluster fathom-dev (no default CNI: Cilium + Hubble) + all addons + samples
k8s/cluster.sh up minimal    # cert-manager + KEDA only
k8s/cluster.sh apply         # re-apply after editing manifests
k8s/cluster.sh down          # blow it away; `up` recreates it identically
k8s/cluster.sh up-cilium     # throwaway kind cluster fathom-cilium with the same Cilium/Hubble stack
k8s/cluster.sh down-cilium   # delete fathom-cilium
```

- `addons/<name>` — cert-manager, keda, knative (+kourier), gatekeeper, trivy (Helm), velero (Helm + RustFS store), flux (v2 controllers; the fathom-dev sandbox podinfo HelmRelease needs it), argocd (v3.5.3 upstream install; `samples/argocd.yaml` adds a synced guestbook app and a deliberately broken one), grafana (13.2.2, admin/`fathom-dev`, Prometheus datasource for the `prometheus` metrics stack; port-forward `svc/grafana 3000` in namespace `grafana`), cilium (Helm; installed at create time by `up` and `up-cilium`, never by an overlay — it needs a cluster with no other CNI)
- `overlays/{full,minimal}` — which addons to install
- `samples/` — workloads/CRs that populate every pane (apply after addons; CRDs must exist)

Add anything else you want to test as a new dir with a `kustomization.yaml`. Helm charts use `--enable-helm`.
Requires: docker, kind, kubectl (with kustomize), helm.

## Hosted mode (spec 031)

```
PROVIDER=rauthy   k8s/cluster.sh up-hosted   # kind cluster fathom-oidc-rauthy: OIDC API server + ingress-nginx + Rauthy + RBAC fixtures
PROVIDER=keycloak k8s/cluster.sh up-hosted   # kind cluster fathom-oidc-keycloak (API-server OIDC flags are fixed at create time)
PROVIDER=... k8s/cluster.sh install-hosted          # rebuild + kind load the image, helm upgrade --install (SKIP_BUILD=1 to reuse)
PROVIDER=... k8s/cluster.sh test-hosted-lifecycle   # helm upgrade (both access models) + uninstall; asserts nothing is left
PROVIDER=... k8s/cluster.sh down-hosted
PROVIDER=... [FATHOM_HOSTED_IP=<node ip>] pnpm -C ui e2e:hosted   # live login/RBAC/exec/port-forward suite
```

`up-hosted` ends with `helm upgrade --install fathom deploy/helm/fathom -n fathom -f overlays/hosted/values-$PROVIDER.yaml`
(image `HOSTED_IMAGE`, default `fathom-server:hosted-dev`, built from `deploy/Dockerfile` and `kind load`ed; `SKIP_INSTALL=1`
stops after the IdP). It waits for the rollout and for `/readyz` through the ingress, then prints the users.

- Hostnames `auth.localtest.me`, `keycloak.localtest.me`, `fathom.localtest.me` resolve to 127.0.0.1 publicly; host ports 80/443 are bound, so run one provider cluster at a time.
- DNS: `*.localtest.me` is public wildcard DNS for 127.0.0.1, so a browser on the host reaches the ingress through the host
  ports. Inside the cluster the same names must reach ingress-nginx instead: the API server (host network) via node `/etc/hosts`
  entries, pods (fathom-server doing OIDC discovery/token calls) via CoreDNS `rewrite` rules. The issuer URL is therefore identical
  for the browser, fathom-server and the API server, which is what OIDC requires.
- Host ports: `HOST_PORTS=auto` (default) binds 80/443 only when they are free. When another hosted cluster owns them, the
  cluster is created without host ports and is reached on the kind node's docker IP (printed by the script): Chromium via
  `--host-resolver-rules` (set `FATHOM_HOSTED_IP` for `e2e:hosted`), curl via `--resolve name:443:<ip>`. This lets a Rauthy
  and a Keycloak cluster run side by side.
- CA: the throwaway CA signs one leaf for all three names (secret `idp-tls` in the IdP namespace and in `fathom`). fathom-server
  trusts it through Helm `oidc.caSecret` (secret `fathom-oidc-ca`, created by the script); the API server through
  `--oidc-ca-file`; browsers either import `k8s/.tmp/ca.crt` or (the e2e suite) ignore certificate errors.
- Provider differences are values only (`overlays/hosted/values-<provider>.yaml`, FR-020). Keycloak rejects unknown scopes, so
  its values drop the `groups` scope (the realm's group mapper adds the claim).
- `.tmp/` (git-ignored) holds a throwaway CA + leaf cert; the script mounts the CA into the API server (`--oidc-ca-file`), injects node `/etc/hosts` entries (API server) and CoreDNS `rewrite` rules (pods) pointing the names at ingress-nginx.
- Issuers: Rauthy `https://auth.localtest.me/auth/v1/` (trailing slash is part of the issuer), Keycloak `https://keycloak.localtest.me/realms/fathom`. Client `fathom` (confidential, PKCE S256, 60 s token lifetime), secret in `addons/<provider>`.
- OIDC seed (spec 045): `samples/hosted-oidc-seed.yaml` (in the hosted overlay) is the seeded issuer as an object --
  Secret `fathom/fathom-oidc` with no value rendered, a ConfigMap of the non-secret `FATHOM_OIDC_*`/`FATHOM_CLAIM_*`
  overrides, and a Role/RoleBinding granting `get` on the Secret to `oidc:fathom-admins`. Hosted shows the issuer
  read-only, flagged as seeded, and only to a user the API server would let read that object: the check is a
  `SelfSubjectAccessReview` for `get` and it fails closed, so without the binding nobody reaches the guided path.
  Point the chart at the same object with `--set oidc.existingSecret=fathom-oidc`; `FATHOM_OIDC_CONFIG_OBJECT`
  (default `secret/fathom-oidc`) and `FATHOM_NAMESPACE` name it when it lives elsewhere.
- Users `alice` (group `fathom-viewers`, view everywhere), `bob` (`fathom-editors`, edit in `demo`), `carol` (no group), all with password `FathomDemo#2026-Pw1`; IdP admin `admin@fathom.test`/`admin` with `FathomAdmin#2026-Pw1`. RBAC: `samples/hosted-rbac.yaml`. Dev-only static secrets, disposable clusters only.

## Restricted user

`samples/restricted-user.yaml` is a namespace-scoped identity for checking that Fathom works
without cluster-wide RBAC: ServiceAccount `fathom-dev/fathom-restricted` with `edit` in
`fathom-dev` and `demo`, `view` in `default`, and nothing cluster-scoped (it cannot list
namespaces, nodes, CRDs, or any kind across all namespaces). It is not in `kustomization.yaml`;
apply it on demand, on kind only, and remove it afterwards.

```
ADMIN=$(mktemp); kind get kubeconfig --name fathom-dev > "$ADMIN"
kubectl --kubeconfig "$ADMIN" apply -f k8s/samples/restricted-user.yaml
# Kubeconfig for the SA (keep it out of the repo; never paste the token anywhere):
KC=$(mktemp); server=$(kubectl --kubeconfig "$ADMIN" config view --minify -o jsonpath='{.clusters[0].cluster.server}')
kubectl --kubeconfig "$ADMIN" config view --minify --raw -o jsonpath='{.clusters[0].cluster.certificate-authority-data}' | base64 -d > "$KC.ca"
kubectl --kubeconfig "$KC" config set-cluster kind-fathom-dev --server="$server" --certificate-authority="$KC.ca" --embed-certs=true
kubectl --kubeconfig "$KC" config set-credentials fathom-restricted \
  --token="$(kubectl --kubeconfig "$ADMIN" -n fathom-dev create token fathom-restricted --duration=12h)"
kubectl --kubeconfig "$KC" config set-context fathom-restricted --cluster=kind-fathom-dev \
  --user=fathom-restricted --namespace=fathom-dev
kubectl --kubeconfig "$KC" config use-context fathom-restricted; rm -f "$KC.ca"
# Live walk of every nav section and tool as that user (skips when the variable is unset):
FATHOM_RESTRICTED_KUBECONFIG=$KC FATHOM_LIVE_CONTEXT=fathom-restricted \
  pnpm -C ui e2e:restricted --workers=1
# Remove:
kubectl --kubeconfig "$ADMIN" delete sa,rolebinding -A -l fathom.dev/restricted-user=true
rm -f "$KC" "$ADMIN"
```

What such a user should see: namespaced lists hold the namespaces they reach (the kubeconfig
context's namespace, plus any typed into the namespace picker's "Use namespace" entry);
cluster-scoped sections (Nodes, PersistentVolumes, ClusterRoles, CRDs, webhooks, integrations
that scan CRDs cluster-wide) show "Not permitted"; no error toasts; background pollers log the
denial once and back off.

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
- Image: `docker build -f deploy/Dockerfile --target fathom-daemon -t fathom-daemon:dev .` Another tag: `DAEMON_IMAGE=fathom-daemon:mytag`, which `up-daemon` swaps into the Deployment.
- Warm snapshots: `/v1/snapshot/{pods,events}` serve the reflector stores (filtered per caller). With the daemon preference on `auto`, the app's pod/event lists take the snapshot and then WATCH from its `resourceVersion`. It falls back to a direct LIST when the daemon is absent, unsynced, or the snapshot is 30 s old or more. Check: `kubectl get --raw /apis/daemon.fathom.dev/v1/v1/snapshot/pods | jq '{resourceVersion, ageMs, n: (.items|length)}'`.
- `test-daemon.sh` also runs the two-user topology check: alice (view on `daemon-demo`) sees only `daemon-demo` nodes/edges, bob sees none, and a `scale` reaches alice's stream in 5 s or less (SC-007).
- Scale fixture (benchmarks in `specs/036-incluster-daemon/research.md`): use a dedicated cluster (e.g. `CLUSTER=fathom-daemon-bench`), run `k8s/bench-fixture.sh` ([kwok](https://kwok.sigs.k8s.io) v0.8.0 fake nodes plus 5,000 pods pinned to them). kindnet's 50Mi limit OOMs at that size, so raise it (`kubectl -n kube-system set resources ds/kindnet --limits=memory=512Mi`).

## Metrics / observability stacks (spec 036)

Selectable stacks for testing Fathom's metrics sources (today the daemon's ring reads `metrics.k8s.io`; PromQL adapters come later).

```
k8s/cluster.sh up-metrics <stack>     # metrics-server | prometheus | victoriametrics | lgtm-lite | openobserve | thanos
k8s/cluster.sh down-metrics <stack|all>
k8s/test-metrics.sh <stack>           # install, wait, assert the query surface, tear down, assert nothing is left (KEEP=1 to keep)
```

- Each stack lives in namespace `fathom-metrics-<stack>` (overlay `overlays/stack-<stack>`, manifests in `addons/`). Every object, including cluster-scoped ones and the kube-system RoleBinding, carries `app.kubernetes.io/part-of=fathom-metrics` and `fathom.dev/metrics-stack=<stack>`. `down-metrics` deletes the APIService first, then `clusterrole,clusterrolebinding,rolebinding -A`, then the namespaces, all by label. All storage is `emptyDir`, so nothing outlives the namespace.
- Re-running `up-metrics` is idempotent: generated secrets (`rustfs-root`, `openobserve-root`) are reused if present; metrics-server's APIService is deleted before apply.
- Run one stack at a time (scraper stacks share ClusterRole names `fathom-metrics-scraper` / `fathom-metrics-kube-state-metrics`); metrics-server may run beside one other stack.
- Scraping (all stacks except metrics-server): cAdvisor via the API-server node proxy, kube-state-metrics v2.17.0, node-exporter v1.9.1 (`addons/scrape-targets`, one shared `scrape.yml`). Prometheus-format scrape + remote_write is done by vmagent for VictoriaMetrics, Mimir and OpenObserve.
- Query from inside the cluster, e.g. `kubectl -n fathom-metrics-prometheus run q --rm -it --image=curlimages/curl:8.11.1 -- curl -s 'http://prometheus:9090/api/v1/query?query=up'`.
- Env: `CLUSTER` (default fathom-dev), `KEEP=1`, `TIMEOUT` (test data wait, seconds; default 300).

| Stack | Query surface for Fathom | Licence (upstream LICENSE) | RAM requests | Fathom adapter needed | Status on kind |
|---|---|---|---|---|---|
| metrics-server v0.8.0 | `metrics.k8s.io` (`kubectl top`) | Apache-2.0 | 200Mi | none, daemon consumes it today | verified |
| prometheus v3.5.0 (+KSM, node-exporter) | PromQL `http://prometheus:9090/api/v1/query` | Apache-2.0 (all three) | ~350Mi | PromQL adapter | verified |
| victoriametrics v1.122.0 single + vmagent | Prometheus HTTP API `http://victoria-metrics:8428/api/v1/query` (also MetricsQL) | Apache-2.0 | ~320Mi | same PromQL adapter | verified |
| lgtm-lite: Mimir 2.17.0 monolithic + RustFS | Prometheus API under `http://mimir:8080/prometheus/api/v1/query` (single tenant, no auth; multi-tenant needs `X-Scope-OrgID`) | Mimir AGPL-3.0; RustFS Apache-2.0 | ~960Mi | same PromQL adapter (path prefix + optional tenant header) | verified |
| openobserve v0.14.7 | PromQL `http://openobserve:5080/api/default/prometheus/api/v1/query`, basic auth (`openobserve-root`); native SQL search API also exists | AGPL-3.0 | ~700Mi | PromQL adapter with base-path + basic auth | verified (see caveat) |
| thanos v0.39.2 (Prometheus + sidecar + query, RustFS) | PromQL `http://thanos-query:9090/api/v1/query` | Apache-2.0 (RustFS Apache-2.0) | ~800Mi | same PromQL adapter | verified |

Licence policy (spec 036): bundled/sidecar defaults must be permissive, so only metrics-server, Prometheus, VictoriaMetrics, Thanos and RustFS are candidates. Mimir and OpenObserve are AGPL-3.0 and are strictly remote-adapter targets (they are only installed here as separate test workloads, never shipped with Fathom).

Caveats:
- Grafana + Loki + Tempo are NOT packaged as a metrics stack (`addons/grafana` is only a dev-cluster test workload for the Integrations > Grafana pane): Loki and Tempo are logs/traces (not metrics) and Grafana/Loki/Tempo are AGPL-3.0 too; "lgtm-lite" is Mimir only, as the metrics query surface is what Fathom needs.
- OpenObserve logs some `ArrowJsonEncodeError ... __hash__` 400s on remote_write for a few series (it still ingests the cAdvisor/KSM/node series the tests query); treat OpenObserve PromQL coverage as partial until an adapter is tested against more series.
- Thanos: the sidecar only uploads completed 2h blocks, so on a short-lived kind cluster the object store stays empty and Query is served by the live sidecar. Store Gateway / Compactor are not packaged.
- Prometheus retention is 2h (6h for Thanos), VictoriaMetrics 1d (its minimum), Mimir/OpenObserve default.
- Not packaged: Netdata (GPL-3.0 agent, own API, not PromQL-first), Coroot (Apache-2.0 community edition; needs ClickHouse + its own agent, too heavy for a laptop kind cluster).

## Promo shoot (ui/tests/promo)

`FATHOM_LIVE_CONTEXT=kind-fathom-dev pnpm -C ui promo` photographs every pane from this one
context and composes `ui/promo-out/fathom-promo.pdf`. It empties `promo-out/shots/` first and
refuses to build the PDF if any scene produced no shot, so a cluster that is missing a component
fails the run instead of quietly reusing an older photograph of it. The full set needs:

```
k8s/cluster.sh up                                            # Cilium/Hubble + every addon + samples
FATHOM_SANDBOX_CONTEXT=kind-fathom-dev scripts/sandbox.sh up # the labelled fathom-dev namespace, podinfo at revision 2
k8s/cluster.sh up-metrics metrics-server                     # the daemon topology sparkline reads metrics.k8s.io
k8s/cluster.sh up-metrics prometheus                         # the live observability suite auto-connects to it
k8s/cluster.sh up-daemon                                     # the three spec-036 daemon scenes
SKIP_INSTALL=1 PROVIDER=rauthy k8s/cluster.sh up-hosted      # second context for the federation fleet scene
```

The live suite (`pnpm e2e:live` from `ui/`) wants the same cluster: `sandbox.sh up` also installs
the bare `quickstart-bare` Helm release the mutation specs roll back and upgrade, and the
observability specs need the `prometheus` stack above.

The fleet scene names `kind-fathom-oidc-rauthy` directly, so that cluster has to be in the
kubeconfig and reachable; nothing is installed into it by the shoot (the whole run is read-only —
`tests/promo/helpers.ts` drops every write before it reaches the bridge).

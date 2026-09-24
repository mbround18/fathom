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
- Grafana + Loki + Tempo are NOT packaged: Loki and Tempo are logs/traces (not metrics) and Grafana/Loki/Tempo are AGPL-3.0 too; "lgtm-lite" is Mimir only, as the metrics query surface is what Fathom needs.
- OpenObserve logs some `ArrowJsonEncodeError ... __hash__` 400s on remote_write for a few series (it still ingests the cAdvisor/KSM/node series the tests query); treat OpenObserve PromQL coverage as partial until an adapter is tested against more series.
- Thanos: the sidecar only uploads completed 2h blocks, so on a short-lived kind cluster the object store stays empty and Query is served by the live sidecar. Store Gateway / Compactor are not packaged.
- Prometheus retention is 2h (6h for Thanos), VictoriaMetrics 1d (its minimum), Mimir/OpenObserve default.
- Not packaged: Netdata (GPL-3.0 agent, own API, not PromQL-first), Coroot (Apache-2.0 community edition; needs ClickHouse + its own agent, too heavy for a laptop kind cluster).

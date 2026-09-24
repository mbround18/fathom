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

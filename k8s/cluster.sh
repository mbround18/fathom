#!/usr/bin/env bash
# Repeatable local kind cluster for testing Fathom against real CRDs/controllers.
#   k8s/cluster.sh up [overlay]     create cluster (if needed) + apply addons overlay + samples  (default: full)
#   k8s/cluster.sh apply [overlay]  re-apply addons + samples to the existing cluster
#   k8s/cluster.sh samples          apply only samples/
#   k8s/cluster.sh down             delete the cluster (blow it away)
# Env: CLUSTER (default fathom-dev). Every kubectl call is pinned to --context kind-$CLUSTER.
set -euo pipefail
cd "$(dirname "$0")"
CLUSTER="${CLUSTER:-fathom-dev}"
CTX="kind-$CLUSTER"
K=(kubectl --context "$CTX")

build_apply() {  # $1 = kustomize dir; retry because CRDs/webhooks race on first apply
  local dir="$1" i
  for i in 1 2 3 4 5; do
    if kubectl kustomize --enable-helm "$dir" | "${K[@]}" apply --server-side --force-conflicts -f - ; then return 0; fi
    echo "apply attempt $i failed; retrying in 15s" >&2; sleep 15
  done
  return 1
}

wait_ready() {
  local ns
  for ns in cert-manager keda knative-serving kourier-system gatekeeper-system trivy-system velero; do
    "${K[@]}" get ns "$ns" >/dev/null 2>&1 && "${K[@]}" -n "$ns" wait --for=condition=Available deploy --all --timeout=300s || true
  done
}

case "${1:-}" in
  up)
    kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER"
    build_apply "overlays/${2:-full}"; wait_ready; build_apply samples ;;
  apply) build_apply "overlays/${2:-full}"; wait_ready; build_apply samples ;;
  samples) build_apply samples ;;
  down) kind delete cluster --name "$CLUSTER" ;;
  *) sed -n 2,7p "$0"; exit 2 ;;
esac

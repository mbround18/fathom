#!/usr/bin/env bash
# Disposable local kind cluster for exercising Fathom's CRD-backed panes (Knative, Gatekeeper,
# Trivy operator, Velero, cert-manager, KEDA) without touching a real cluster.
#
#   scripts/dev-cluster.sh up     [--name fathom-dev] [--addons all|none|a,b,...]
#   scripts/dev-cluster.sh addons [--name fathom-dev] --addons cert-manager,keda
#   scripts/dev-cluster.sh down   [--name fathom-dev]
#
# Addons: cert-manager, keda, knative, gatekeeper, trivy, velero (CRDs only, except gatekeeper
# which installs its bundled manifest so ConstraintTemplates work).
#
# Safety: only ever runs `kind create/delete cluster --name <name>` and kubectl pinned with an
# explicit --context kind-<name>; never uses or changes any other kubeconfig context. Arguments
# are always passed as arrays (no eval, no shell interpolation of user input).
set -euo pipefail

NAME="fathom-dev"
ADDONS="all"
CMD="${1:-}"
[[ $# -gt 0 ]] && shift
while [[ $# -gt 0 ]]; do
  case "$1" in
    --name) NAME="${2:?--name needs a value}"; shift 2 ;;
    --addons) ADDONS="${2:?--addons needs a value}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# RFC 1123 label, same rule as src/backend/kind.rs::validate_cluster_name.
if ! [[ "$NAME" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; then
  echo "invalid cluster name '$NAME' (lowercase alphanumerics and '-', 1-63 chars)" >&2
  exit 2
fi
CTX="kind-${NAME}"
KC=(kubectl --context "$CTX")

declare -A URLS=(
  [cert-manager]="https://github.com/cert-manager/cert-manager/releases/download/v1.15.3/cert-manager.crds.yaml"
  [keda]="https://github.com/kedacore/keda/releases/download/v2.15.1/keda-2.15.1-crds.yaml"
  [knative]="https://github.com/knative/serving/releases/download/knative-v1.15.2/serving-crds.yaml"
  [gatekeeper]="https://raw.githubusercontent.com/open-policy-agent/gatekeeper/v3.17.1/deploy/gatekeeper.yaml"
)
# CRD directories fetched through the GitHub contents API.
declare -A CRD_DIRS=(
  [trivy]="https://api.github.com/repos/aquasecurity/trivy-operator/contents/deploy/helm/crds?ref=v0.22.0"
  [velero]="https://api.github.com/repos/vmware-tanzu/helm-charts/contents/charts/velero/crds"
)

apply_url() { "${KC[@]}" apply --server-side --force-conflicts -f "$1"; }

apply_crd_dir() {
  local listing url
  listing="$(curl -fsSL "$1")"
  while IFS= read -r url; do
    [[ -n "$url" ]] && apply_url "$url"
  done < <(grep -o '"download_url": *"[^"]*\.ya\?ml"' <<<"$listing" | sed 's/.*: *"\(.*\)"/\1/')
}

install_addon() {
  echo "==> $1"
  if [[ -n "${URLS[$1]:-}" ]]; then apply_url "${URLS[$1]}"
  elif [[ -n "${CRD_DIRS[$1]:-}" ]]; then apply_crd_dir "${CRD_DIRS[$1]}"
  else echo "unknown addon: $1" >&2; exit 2; fi
}

install_addons() {
  local list="$ADDONS" a
  [[ "$list" == "none" ]] && return 0
  [[ "$list" == "all" ]] && list="cert-manager,keda,knative,gatekeeper,trivy,velero"
  IFS=',' read -ra picked <<<"$list"
  for a in "${picked[@]}"; do install_addon "$a"; done
}

case "$CMD" in
  up)
    if kind get clusters 2>/dev/null | grep -qx "$NAME"; then
      echo "kind cluster '$NAME' already exists; installing addons only"
    else
      kind create cluster --name "$NAME"
    fi
    install_addons
    echo "Ready. Context: $CTX (select it in Fathom)."
    ;;
  addons) install_addons ;;
  down) kind delete cluster --name "$NAME" ;;
  *) sed -n '2,12p' "$0"; exit 2 ;;
esac

#!/usr/bin/env bash
# Repeatable local kind cluster for testing Fathom against real CRDs/controllers.
#   k8s/cluster.sh up [overlay]     create cluster (if needed) + apply addons overlay + samples  (default: full)
#   k8s/cluster.sh apply [overlay]  re-apply addons + samples to the existing cluster
#   k8s/cluster.sh samples          apply only samples/
#   k8s/cluster.sh down             delete the cluster (blow it away)
#   PROVIDER=rauthy|keycloak k8s/cluster.sh up-hosted    OIDC-aware kind cluster + ingress + IdP + RBAC fixtures (spec 031)
#   PROVIDER=... k8s/cluster.sh down-hosted              delete that cluster (+ generated certs when none remain)
# Env: CLUSTER (default fathom-dev; up-hosted defaults to fathom-oidc-$PROVIDER). Every kubectl call is pinned
# to --context kind-$CLUSTER. Hosted clusters bind host ports 80/443, so run one at a time.
set -euo pipefail
cd "$(dirname "$0")"
PROVIDER="${PROVIDER:-rauthy}"
case "${1:-}" in up-hosted|down-hosted) CLUSTER="${CLUSTER:-fathom-oidc-$PROVIDER}" ;; esac
CLUSTER="${CLUSTER:-fathom-dev}"
CTX="kind-$CLUSTER"
K=(kubectl --context "$CTX")

build_apply() {  # $1 = kustomize dir; retry because CRDs/webhooks race on first apply
  local dir="$1" i
  for i in 1 2 3 4 5; do
    if kubectl kustomize --enable-helm --load-restrictor LoadRestrictionsNone "$dir" | "${K[@]}" apply --server-side --force-conflicts -f - ; then return 0; fi
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

# ---- hosted (spec 031) --------------------------------------------------------------------------------
TMP="$PWD/.tmp"   # throwaway CA + leaf certs (git-ignored)
case "$PROVIDER" in
  rauthy)   IDP_HOST=auth.localtest.me;     ISSUER="https://auth.localtest.me/auth/v1/"; IDP_NS=rauthy ;;
  keycloak) IDP_HOST=keycloak.localtest.me; ISSUER="https://keycloak.localtest.me/realms/fathom"; IDP_NS=keycloak ;;
  *) echo "PROVIDER must be rauthy or keycloak" >&2; exit 2 ;;
esac
HOSTED_HOSTS=(auth.localtest.me keycloak.localtest.me fathom.localtest.me)

gen_certs() {  # one CA + one leaf covering all hosted hostnames; reused across clusters
  mkdir -p "$TMP"; printf '*\n' > "$TMP/.gitignore"
  [ -f "$TMP/ca.crt" ] && [ -f "$TMP/tls.crt" ] && return 0
  local san=""; for h in "${HOSTED_HOSTS[@]}"; do san+="DNS:$h,"; done
  openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=fathom-dev-ca" \
    -keyout "$TMP/ca.key" -out "$TMP/ca.crt" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -subj "/CN=localtest.me" -keyout "$TMP/tls.key" -out "$TMP/tls.csr" 2>/dev/null
  printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' "${san%,}" > "$TMP/ext.cnf"
  openssl x509 -req -in "$TMP/tls.csr" -CA "$TMP/ca.crt" -CAkey "$TMP/ca.key" -CAcreateserial -days 30 \
    -extfile "$TMP/ext.cnf" -out "$TMP/tls.crt" 2>/dev/null
}

hosted_up() {
  gen_certs
  if ! kind get clusters 2>/dev/null | grep -qx "$CLUSTER"; then
    sed "s#__ISSUER__#$ISSUER#; s#__CA_DIR__#$TMP#" kind-oidc.yaml | kind create cluster --name "$CLUSTER" --config -
  fi
  # names must resolve to the ingress from the API server (host network on the node): node /etc/hosts
  local node ip h
  node="$CLUSTER-control-plane"
  ip=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$node")
  for h in "${HOSTED_HOSTS[@]}"; do
    docker exec "$node" grep -q " $h\$" /etc/hosts || docker exec "$node" sh -c "echo '$ip $h' >> /etc/hosts"
  done
  # ...and from pods: CoreDNS rewrites the names to the ingress controller Service
  local rules="" ; for h in "${HOSTED_HOSTS[@]}"; do rules+="    rewrite name $h ingress-nginx-controller.ingress-nginx.svc.cluster.local\n"; done
  "${K[@]}" -n kube-system get cm coredns -o jsonpath='{.data.Corefile}' > "$TMP/Corefile"
  if ! grep -q "rewrite name auth.localtest.me" "$TMP/Corefile"; then
    sed "s#^\(\s*\)ready\$#\1ready\n${rules%\\n}#" "$TMP/Corefile" > "$TMP/Corefile.new"
    "${K[@]}" -n kube-system create cm coredns --from-file=Corefile="$TMP/Corefile.new" --dry-run=client -o yaml \
      | "${K[@]}" -n kube-system apply -f -
    "${K[@]}" -n kube-system rollout restart deploy/coredns
  fi
  # TLS secrets (before the Ingresses reference them)
  local ns
  for ns in "$IDP_NS" fathom; do
    "${K[@]}" create ns "$ns" --dry-run=client -o yaml | "${K[@]}" apply -f -
    "${K[@]}" -n "$ns" create secret tls idp-tls --cert="$TMP/tls.crt" --key="$TMP/tls.key" --dry-run=client -o yaml | "${K[@]}" apply -f -
  done
  build_apply addons/ingress-nginx
  "${K[@]}" -n ingress-nginx wait --for=condition=Available deploy --all --timeout=300s
  build_apply "overlays/hosted-$PROVIDER"
  "${K[@]}" -n "$IDP_NS" wait --for=condition=Available deploy --all --timeout=600s
  echo "cluster $CLUSTER ready. Issuer: $ISSUER  (CA: $TMP/ca.crt)"
  echo "users alice/bob/carol password: FathomDemo#2026-Pw1   (IdP admin: see addons/$PROVIDER)"
}

# ---- in-cluster daemon (spec 036) ---------------------------------------------------------------------
DAEMON_IMAGE="${DAEMON_IMAGE:-fathom-daemon:dev}"
daemon_up() {  # $1 = overlay (daemon | daemon-namespaced)
  kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER"
  if [ -z "${SKIP_BUILD:-}" ]; then
    docker build -f ../deploy/Dockerfile --target fathom-daemon -t "$DAEMON_IMAGE" ..
  fi
  kind load docker-image "$DAEMON_IMAGE" --name "$CLUSTER"
  KUBECTL="${K[*]}" addons/fathom-daemon/gen-certs.sh
  build_apply "overlays/${1:-daemon}"
  KUBECTL="${K[*]}" addons/fathom-daemon/gen-certs.sh --patch   # re-applied after every apply (apply resets it)
  "${K[@]}" apply -f samples/daemon-events.yaml
  "${K[@]}" -n fathom-daemon rollout status deploy/fathom-daemon --timeout=120s || true
}
daemon_down() {  # uninstall by label: leaves no fathom-daemon object (cluster-scoped, kube-system, namespace)
  # APIService first and on its own: a stale (unavailable) aggregated group makes namespace deletion hang
  "${K[@]}" delete apiservice -l app.kubernetes.io/name=fathom-daemon --ignore-not-found
  "${K[@]}" delete -f samples/daemon-events.yaml --ignore-not-found --wait=false || true
  "${K[@]}" delete clusterrole,clusterrolebinding,role,rolebinding,namespace -A \
    -l app.kubernetes.io/name=fathom-daemon --ignore-not-found
}

case "${1:-}" in
  up-daemon) daemon_up "${2:-daemon}" ;;
  down-daemon) daemon_down ;;
  up-hosted) hosted_up ;;
  down-hosted) kind delete cluster --name "$CLUSTER" ;;
  up)
    kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER"
    build_apply "overlays/${2:-full}"; wait_ready; build_apply samples ;;
  apply) build_apply "overlays/${2:-full}"; wait_ready; build_apply samples ;;
  samples) build_apply samples ;;
  down) kind delete cluster --name "$CLUSTER" ;;
  *) sed -n 2,7p "$0"; echo "  up-daemon [daemon|daemon-namespaced] / down-daemon   in-cluster daemon (spec 036)"; exit 2 ;;
esac

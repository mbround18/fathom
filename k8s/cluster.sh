#!/usr/bin/env bash
# Repeatable local kind cluster for testing Fathom against real CRDs/controllers.
#   k8s/cluster.sh up [overlay]     create cluster (if needed) + apply addons overlay + samples  (default: full)
#   k8s/cluster.sh apply [overlay]  re-apply addons + samples to the existing cluster
#   k8s/cluster.sh samples          apply only samples/
#   k8s/cluster.sh down             delete the cluster (blow it away)
#   k8s/cluster.sh up-cilium / down-cilium   separate kind cluster fathom-cilium (no default CNI) + Cilium/Hubble + traffic
#   PROVIDER=rauthy|keycloak k8s/cluster.sh up-hosted    OIDC-aware kind cluster + ingress + IdP + RBAC fixtures (spec 031)
#   k8s/cluster.sh up-metrics <stack> / down-metrics <stack|all>   metrics stacks (see README); k8s/test-metrics.sh <stack>
#   PROVIDER=... k8s/cluster.sh install-hosted           (re)build + kind load the server image, helm upgrade --install fathom
#   PROVIDER=... k8s/cluster.sh test-hosted-lifecycle    helm upgrade/uninstall e2e: asserts no leftover cluster-scoped objects
#   PROVIDER=... k8s/cluster.sh down-hosted              delete that cluster
# Env: CLUSTER (default fathom-dev; up-hosted defaults to fathom-oidc-$PROVIDER). Every kubectl call is pinned
# to --context kind-$CLUSTER. Hosted env: HOSTED_IMAGE (default fathom-server:hosted-dev), SKIP_BUILD=1 (reuse
# the image), SKIP_INSTALL=1 (IdP only), HOST_PORTS=1|0|auto (bind host 80/443; auto = only when free).
set -euo pipefail
cd "$(dirname "$0")"
PROVIDER="${PROVIDER:-rauthy}"
case "${1:-}" in up-hosted|down-hosted|install-hosted|test-hosted-lifecycle) CLUSTER="${CLUSTER:-fathom-oidc-$PROVIDER}" ;; esac
case "${1:-}" in up-cilium|down-cilium) CLUSTER="${CLUSTER:-fathom-cilium}" ;; esac
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
  for ns in cert-manager keda knative-serving kourier-system gatekeeper-system trivy-system velero argocd grafana; do
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
    local ports="${HOST_PORTS:-auto}" strip=""
    if [ "$ports" = auto ]; then
      if ss -ltnH 2>/dev/null | awk '{print $4}' | grep -Eq '[:.](80|443)$'; then ports=0; else ports=1; fi
    fi
    # without host ports (another hosted cluster owns 80/443) the node is reached on its docker IP; see README
    [ "$ports" = 1 ] || { strip='/hostPort: \(80\|443\)/d'; echo "host ports 80/443 not bound (HOST_PORTS=$ports)"; }
    sed "s#__ISSUER__#$ISSUER#; s#__CA_DIR__#$TMP#; $strip" kind-oidc.yaml | kind create cluster --name "$CLUSTER" --config -
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
  [ -n "${SKIP_INSTALL:-}" ] || hosted_install
  echo "cluster $CLUSTER ready. Issuer: $ISSUER  (CA: $TMP/ca.crt)"
  echo "users alice/bob/carol password: FathomDemo#2026-Pw1   (IdP admin: see addons/$PROVIDER)"
  hosted_access_hint
}

HOSTED_IMAGE="${HOSTED_IMAGE:-fathom-server:hosted-dev}"
HELM=(helm --kube-context "$CTX" -n fathom)
node_ip() { docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$CLUSTER-control-plane"; }
hosted_access_hint() {
  local ip; ip=$(node_ip)
  if docker port "$CLUSTER-control-plane" 443 >/dev/null 2>&1; then
    echo "open https://fathom.localtest.me (host ports bound)"
  else
    echo "host ports not bound: reach the ingress on the node IP $ip, e.g."
    echo "  curl --cacert $TMP/ca.crt --resolve fathom.localtest.me:443:$ip https://fathom.localtest.me/readyz"
    echo "  FATHOM_HOSTED_IP=$ip PROVIDER=$PROVIDER pnpm -C ui e2e:hosted"
  fi
}
hosted_curl() {  # $1 = path on fathom.localtest.me; prints the HTTP status
  curl -s -o /dev/null -w '%{http_code}' --cacert "$TMP/ca.crt" --resolve "fathom.localtest.me:443:$(node_ip)" \
    "https://fathom.localtest.me$1"
}
hosted_install() {  # build + load image, CA secret, helm upgrade --install with the provider values
  if [ -z "${SKIP_BUILD:-}" ]; then
    docker build -f ../deploy/Dockerfile -t "$HOSTED_IMAGE" ..
  fi
  kind load docker-image "$HOSTED_IMAGE" --name "$CLUSTER"
  "${K[@]}" create ns fathom --dry-run=client -o yaml | "${K[@]}" apply -f -
  "${K[@]}" -n fathom create secret generic fathom-oidc-ca --from-file=ca.crt="$TMP/ca.crt" --dry-run=client -o yaml \
    | "${K[@]}" apply -f -
  "${HELM[@]}" upgrade --install fathom ../deploy/helm/fathom -f "overlays/hosted/values-$PROVIDER.yaml" \
    --set image.repository="${HOSTED_IMAGE%:*}" --set image.tag="${HOSTED_IMAGE##*:}" "$@" --wait --timeout 5m
  "${K[@]}" -n fathom rollout status deploy/fathom-fathom --timeout=180s
  local i; for i in $(seq 1 30); do [ "$(hosted_curl /readyz)" = 200 ] && break; sleep 2; done
  [ "$(hosted_curl /readyz)" = 200 ] || { echo "fathom /readyz not 200 through the ingress" >&2; return 1; }
  echo "fathom installed: https://fathom.localtest.me ($PROVIDER, image $HOSTED_IMAGE)"
}

# helm upgrade/uninstall e2e (T047): the impersonation model is the only one with cluster-scoped objects,
# so switch to it, back, and to it again before uninstalling; nothing labelled for the release may remain.
hosted_lifecycle() {
  local sel=app.kubernetes.io/instance=fathom left
  fail() { echo "FAIL: $*" >&2; exit 1; }
  cluster_scoped() { "${K[@]}" get clusterrole,clusterrolebinding -l "$sel" -o name; }
  "${HELM[@]}" status fathom >/dev/null 2>&1 || SKIP_BUILD=1 hosted_install
  echo "== upgrade: accessModel=impersonation"
  "${HELM[@]}" upgrade fathom ../deploy/helm/fathom --reuse-values --set accessModel=impersonation --wait --timeout 5m
  "${K[@]}" -n fathom rollout status deploy/fathom-fathom --timeout=180s
  [ "$(cluster_scoped | wc -l)" -eq 2 ] || fail "impersonation should create 1 ClusterRole + 1 ClusterRoleBinding"
  [ "$(hosted_curl /readyz)" = 200 ] || fail "not ready after upgrade to impersonation"
  echo "== upgrade: accessModel=passthrough (cluster-scoped objects must be pruned)"
  "${HELM[@]}" upgrade fathom ../deploy/helm/fathom --reuse-values --set accessModel=passthrough --wait --timeout 5m
  "${K[@]}" -n fathom rollout status deploy/fathom-fathom --timeout=180s
  left=$(cluster_scoped); [ -z "$left" ] || fail "left after downgrade to passthrough: $left"
  [ "$(hosted_curl /readyz)" = 200 ] || fail "not ready after upgrade to passthrough"
  echo "== uninstall (from impersonation)"
  "${HELM[@]}" upgrade fathom ../deploy/helm/fathom --reuse-values --set accessModel=impersonation --wait --timeout 5m
  "${HELM[@]}" uninstall fathom --wait --timeout 5m
  left=$(cluster_scoped); [ -z "$left" ] || fail "cluster-scoped leftovers after uninstall: $left"
  left=$("${K[@]}" -n fathom get all,ingress,secret,serviceaccount,configmap,pvc,networkpolicy,pdb -l "$sel" -o name)
  [ -z "$left" ] || fail "namespaced leftovers after uninstall: $left"
  echo "== reinstall (restores the passthrough install)"
  SKIP_BUILD=1 hosted_install
  echo "lifecycle OK"
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
  # idempotent re-run: a previously patched APIService (caBundle set) rejects the base manifest's insecureSkipTLSVerify
  "${K[@]}" delete apiservice v1.daemon.fathom.dev --ignore-not-found
  build_apply "overlays/${1:-daemon}"
  KUBECTL="${K[*]}" addons/fathom-daemon/gen-certs.sh --patch   # re-applied after every apply (apply resets it)
  "${K[@]}" apply -f samples/daemon-events.yaml
  # the manifest pins fathom-daemon:dev; a different tag (e.g. a benchmark build) is swapped in here
  [ "$DAEMON_IMAGE" = fathom-daemon:dev ] || "${K[@]}" -n fathom-daemon set image deploy/fathom-daemon fathom-daemon="$DAEMON_IMAGE"
  "${K[@]}" -n fathom-daemon rollout status deploy/fathom-daemon --timeout=120s || true
}
daemon_down() {  # uninstall by label: leaves no fathom-daemon object (cluster-scoped, kube-system, namespace)
  # APIService first and on its own: a stale (unavailable) aggregated group makes namespace deletion hang
  "${K[@]}" delete apiservice -l app.kubernetes.io/name=fathom-daemon --ignore-not-found
  "${K[@]}" delete -f samples/daemon-events.yaml --ignore-not-found --wait=false || true
  "${K[@]}" delete clusterrole,clusterrolebinding,role,rolebinding,namespace -A \
    -l app.kubernetes.io/name=fathom-daemon --ignore-not-found
}

# ---- metrics / observability stacks (spec 036 adapters) -----------------------------------------------
METRICS_STACKS="metrics-server prometheus victoriametrics lgtm-lite openobserve thanos"
SEL=app.kubernetes.io/part-of=fathom-metrics
ensure_secret() {  # $1 ns, $2 name, rest key=value ("@gen" = random). Reused when it already exists (stable across re-runs)
  local ns="$1" name="$2" args=() kv; shift 2
  "${K[@]}" -n "$ns" get secret "$name" >/dev/null 2>&1 && return 0
  for kv in "$@"; do
    case "$kv" in *=@gen) kv="${kv%=@gen}=$(openssl rand -hex 12)" ;; esac
    args+=(--from-literal="$kv")
  done
  "${K[@]}" -n "$ns" create secret generic "$name" "${args[@]}"
  "${K[@]}" -n "$ns" label secret "$name" "$SEL" --overwrite
}
metrics_up() {  # $1 = stack
  local stack="${1:-}" ns pw
  case " $METRICS_STACKS " in *" $stack "*) ;; *) echo "stack must be one of: $METRICS_STACKS" >&2; exit 2 ;; esac
  ns="fathom-metrics-$stack"
  kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER"
  if [ "$stack" = metrics-server ]; then
    # idempotent re-run: drop our APIService first so a stale (Unavailable) one cannot block the apply
    "${K[@]}" delete apiservice -l "$SEL" --ignore-not-found
  else
    "${K[@]}" create ns "$ns" --dry-run=client -o yaml | "${K[@]}" apply -f -
    "${K[@]}" label ns "$ns" "$SEL" "fathom.dev/metrics-stack=$stack" --overwrite
    case "$stack" in
      lgtm-lite|thanos) ensure_secret "$ns" rustfs-root access-key=@gen secret-key=@gen ;;
      openobserve)
        ensure_secret "$ns" openobserve-root email=root@fathom.local password=@gen
        pw=$("${K[@]}" -n "$ns" get secret openobserve-root -o jsonpath='{.data.password}' | base64 -d)
        ensure_secret "$ns" remote-write-auth user=root@fathom.local password="$pw" ;;
    esac
  fi
  build_apply "overlays/stack-$stack"
  "${K[@]}" -n "$ns" wait --for=condition=Available deploy --all --timeout=300s
  "${K[@]}" -n "$ns" get ds -o name | while read -r d; do "${K[@]}" -n "$ns" rollout status "$d" --timeout=180s; done
  [ "$stack" != metrics-server ] || "${K[@]}" wait --for=condition=Available apiservice/v1beta1.metrics.k8s.io --timeout=120s
  echo "metrics stack $stack ready in namespace $ns"
}
metrics_down() {  # $1 = stack | all; uninstall by label; leaves nothing behind
  local sel="$SEL"
  case "${1:-all}" in all) ;; *) sel="$SEL,fathom.dev/metrics-stack=$1" ;; esac
  # APIService first and on its own: a stale (unavailable) aggregated group makes namespace deletion hang
  "${K[@]}" delete apiservice -l "$sel" --ignore-not-found
  "${K[@]}" delete clusterrole,clusterrolebinding,rolebinding -A -l "$sel" --ignore-not-found
  "${K[@]}" delete namespace -l "$sel" --ignore-not-found
}

# ---- Cilium + Hubble (Integrations > Hubble) ----------------------------------------------------------
# Cilium must replace kindnet, so it gets its own cluster (CLUSTER defaults to fathom-cilium here).
cilium_up() {
  kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER" --config kind-cilium.yaml
  build_apply addons/cilium
  "${K[@]}" -n kube-system rollout status ds/cilium --timeout=300s
  "${K[@]}" -n kube-system rollout status deploy/hubble-relay --timeout=300s
  "${K[@]}" apply -f samples/hubble-traffic.yaml
  echo "cluster $CLUSTER ready: select context $CTX in Fathom and open Integrations > Hubble"
}

case "${1:-}" in
  up-cilium) cilium_up ;;
  down-cilium) kind delete cluster --name "$CLUSTER" ;;
  up-metrics) metrics_up "${2:-}" ;;
  down-metrics) metrics_down "${2:-all}" ;;
  up-daemon) daemon_up "${2:-daemon}" ;;
  down-daemon) daemon_down ;;
  up-hosted) hosted_up ;;
  install-hosted) hosted_install ;;
  test-hosted-lifecycle) hosted_lifecycle ;;
  down-hosted) kind delete cluster --name "$CLUSTER" ;;
  up)
    kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER"
    build_apply "overlays/${2:-full}"; wait_ready; build_apply samples ;;
  apply) build_apply "overlays/${2:-full}"; wait_ready; build_apply samples ;;
  samples) build_apply samples ;;
  down) kind delete cluster --name "$CLUSTER" ;;
  *) sed -n 2,8p "$0"; echo "  up-daemon [daemon|daemon-namespaced] / down-daemon   in-cluster daemon (spec 036)"
     echo "  up-metrics <stack> / down-metrics <stack|all>   metrics stacks: $METRICS_STACKS"; exit 2 ;;
esac

#!/usr/bin/env bash
# Integration test for a metrics stack: install, wait, verify the query surface, tear down, assert nothing is left.
#   k8s/test-metrics.sh <stack>          stacks: metrics-server prometheus victoriametrics lgtm-lite openobserve thanos
#   KEEP=1 k8s/test-metrics.sh <stack>   leave it installed
#   TIMEOUT=300                          seconds to wait for scraped data to become queryable
# Queries run from a throwaway curl pod inside the stack's namespace (no port-forward, works for basic auth too).
set -euo pipefail
cd "$(dirname "$0")"
STACK="${1:?usage: test-metrics.sh <stack>}"
CLUSTER="${CLUSTER:-fathom-dev}"
K=(kubectl --context "kind-$CLUSTER")
NS="fathom-metrics-$STACK"
SEL=app.kubernetes.io/part-of=fathom-metrics
TIMEOUT="${TIMEOUT:-300}"
ok() { echo "ok: $*"; }
die() { echo "FAIL: $*" >&2; exit 1; }

./cluster.sh up-metrics "$STACK"

# curl_in <curl args...>: run curl in a throwaway pod in the stack namespace, print the body
curl_in() {
  "${K[@]}" -n "$NS" run "q-$RANDOM" --image=curlimages/curl:8.11.1 --rm -i --restart=Never --quiet \
    --labels="$SEL,fathom.dev/metrics-stack=$STACK" -- curl -sf "$@"
}

# wait_for <description> <command...>: poll until the command succeeds
wait_for() {
  local what="$1" i; shift
  for ((i = 0; i < TIMEOUT / 5; i++)); do "$@" >/dev/null 2>&1 && { ok "$what"; return 0; }; sleep 5; done
  die "$what (timed out after ${TIMEOUT}s)"
}

# promql_has <base-url> <query> [curl auth args]: instant query returns status=success with >= 1 series
promql_has() {
  local base="$1" q="$2"; shift 2
  curl_in "$@" --get "$base/api/v1/query" --data-urlencode "query=$q" \
    | jq -e '.status == "success" and (.data.result | length > 0)'
}

case "$STACK" in
  metrics-server)
    wait_for "kubectl top nodes" "${K[@]}" top nodes
    wait_for "kubectl top pods -A" "${K[@]}" top pods -A
    "${K[@]}" get --raw /apis/metrics.k8s.io/v1beta1/nodes | jq -e '.items | length > 0' >/dev/null || die "NodeMetricsList empty"
    ok "metrics.k8s.io NodeMetricsList has items"
    ;;
  prometheus|victoriametrics|lgtm-lite|thanos|openobserve)
    AUTH=()
    case "$STACK" in
      prometheus)      BASE=http://prometheus:9090 ;;
      victoriametrics) BASE=http://victoria-metrics:8428 ;;
      lgtm-lite)       BASE=http://mimir:8080/prometheus ;;
      thanos)          BASE=http://thanos-query:9090 ;;
      openobserve)
        BASE=http://openobserve:5080/api/default/prometheus
        pw=$("${K[@]}" -n "$NS" get secret openobserve-root -o jsonpath='{.data.password}' | base64 -d)
        AUTH=(-u "root@fathom.local:$pw") ;;
    esac
    # cAdvisor (what a Fathom PromQL adapter would use for per-pod CPU) and kube-state-metrics
    wait_for "PromQL container_cpu_usage_seconds_total via $BASE" \
      promql_has "$BASE" container_cpu_usage_seconds_total "${AUTH[@]}"
    wait_for "PromQL container_memory_working_set_bytes" promql_has "$BASE" container_memory_working_set_bytes "${AUTH[@]}"
    wait_for "PromQL kube_pod_info (kube-state-metrics)" promql_has "$BASE" kube_pod_info "${AUTH[@]}"
    wait_for "PromQL node_cpu_seconds_total (node-exporter)" promql_has "$BASE" node_cpu_seconds_total "${AUTH[@]}"
    wait_for "PromQL rate() over cAdvisor counter" \
      promql_has "$BASE" 'sum by (namespace, pod) (rate(container_cpu_usage_seconds_total{container!=""}[1m]))' "${AUTH[@]}"
    if [ "$STACK" = thanos ]; then
      curl_in "$BASE/api/v1/stores" | jq -e '.data.sidecar | length > 0' >/dev/null || die "thanos sidecar not registered as a store"
      ok "thanos query sees the sidecar store"
    fi
    ;;
  *) die "unknown stack $STACK" ;;
esac

if [ -n "${KEEP:-}" ]; then echo "KEEP=1: leaving $STACK installed"; exit 0; fi
./cluster.sh down-metrics "$STACK"
for i in $(seq 1 24); do
  left=$("${K[@]}" get apiservice,clusterrole,clusterrolebinding,namespace -l "$SEL,fathom.dev/metrics-stack=$STACK" -o name 2>/dev/null || true)
  left+=$("${K[@]}" get rolebinding -A -l "$SEL,fathom.dev/metrics-stack=$STACK" -o name 2>/dev/null || true)
  [ -z "$left" ] && break
  sleep 5
done
[ -z "$left" ] || die "objects left behind after down-metrics: $left"
ok "down-metrics left nothing behind"
echo "PASS: $STACK"

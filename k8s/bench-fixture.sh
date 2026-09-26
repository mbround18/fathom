#!/usr/bin/env bash
# Scale fixture for daemon benchmarks (spec 036 T036/T037): kwok fake nodes + N pods that never run containers.
#   CLUSTER=fathom-daemon-bench k8s/bench-fixture.sh          # 50 kwok nodes, 10 ns x 5 deployments x 100 pods = 5,000 pods
#   NODES=20 NS=2 DEPS=5 REPLICAS=100 k8s/bench-fixture.sh     # smaller
#   CLUSTER=... k8s/bench-fixture.sh down                      # delete the bench-* namespaces and kwok nodes
# Use a dedicated cluster; never a shared one. Numbers are recorded in specs/036-incluster-daemon/research.md.
set -euo pipefail
CLUSTER="${CLUSTER:-fathom-daemon-bench}"
KWOK_VERSION="${KWOK_VERSION:-v0.8.0}"
NODES="${NODES:-50}"; NS="${NS:-10}"; DEPS="${DEPS:-5}"; REPLICAS="${REPLICAS:-100}"
k() { kubectl --context "kind-$CLUSTER" "$@"; }

if [ "${1:-}" = down ]; then
  k delete ns -l fathom.dev/bench=1 --wait=false
  k delete node -l type=kwok
  exit 0
fi

kind get clusters 2>/dev/null | grep -qx "$CLUSTER" || kind create cluster --name "$CLUSTER"
# kindnet's default 50Mi limit OOMs once the cluster holds thousands of pods
k -n kube-system set resources ds/kindnet --limits=memory=512Mi,cpu=500m --requests=memory=50Mi,cpu=100m >/dev/null

rel="https://github.com/kubernetes-sigs/kwok/releases/download/$KWOK_VERSION"
k apply -f "$rel/kwok.yaml" >/dev/null
k apply -f "$rel/stage-fast.yaml" >/dev/null
k -n kube-system rollout status deploy/kwok-controller --timeout=180s

for i in $(seq 0 $((NODES - 1))); do
  printf -- '---\napiVersion: v1\nkind: Node\nmetadata:\n  name: kwok-node-%s\n' "$i"
  printf '  annotations: {node.alpha.kubernetes.io/ttl: "0", kwok.x-k8s.io/node: fake}\n'
  printf '  labels: {type: kwok, kubernetes.io/role: agent, kubernetes.io/hostname: kwok-node-%s}\n' "$i"
  printf 'spec:\n  taints: [{key: kwok.x-k8s.io/node, value: fake, effect: NoSchedule}]\n'
  printf 'status:\n  allocatable: {cpu: "64", memory: 256Gi, pods: "200"}\n  capacity: {cpu: "64", memory: 256Gi, pods: "200"}\n'
done | k apply -f - >/dev/null

for n in $(seq 0 $((NS - 1))); do
  printf -- '---\napiVersion: v1\nkind: Namespace\nmetadata: {name: bench-%s, labels: {fathom.dev/bench: "1"}}\n' "$n"
  for d in $(seq 0 $((DEPS - 1))); do
    printf -- '---\napiVersion: apps/v1\nkind: Deployment\nmetadata: {name: fake-%s, namespace: bench-%s}\n' "$d" "$n"
    printf 'spec:\n  replicas: %s\n  selector: {matchLabels: {app: fake-%s}}\n  template:\n' "$REPLICAS" "$d"
    printf '    metadata: {labels: {app: fake-%s, tier: bench}}\n    spec:\n' "$d"
    printf '      nodeSelector: {type: kwok}\n'
    printf '      tolerations: [{key: kwok.x-k8s.io/node, operator: Exists, effect: NoSchedule}]\n'
    printf '      containers: [{name: app, image: registry.example/fake:1, resources: {requests: {cpu: 1m, memory: 1Mi}}}]\n'
  done
done | k apply -f - >/dev/null

want=$((NS * DEPS * REPLICAS))
for _ in $(seq 1 180); do
  running=$(k get pods -A -l tier=bench --field-selector=status.phase=Running --no-headers 2>/dev/null | wc -l)
  [ "$running" -ge "$want" ] && { echo "fixture ready: $running pods on $NODES kwok nodes"; exit 0; }
  sleep 5
done
echo "fixture incomplete: $running/$want pods Running" >&2; exit 1

#!/usr/bin/env bash
# Chart checks: lint, template both models, kubeconform (if present), pass-through has no cluster-scoped objects.
set -euo pipefail
cd "$(dirname "$0")/.."
fail() { echo "FAIL: $*" >&2; exit 1; }
helm lint . -f ci/values-passthrough.yaml
helm lint . -f ci/values-impersonation.yaml
PT=$(helm template t . -f ci/values-passthrough.yaml)
IM=$(helm template t . -f ci/values-impersonation.yaml)
if command -v kubeconform >/dev/null; then
  echo "$PT" | kubeconform -strict -summary -
  echo "$IM" | kubeconform -strict -summary -
else echo "kubeconform not installed; skipped"; fi
if echo "$PT" | grep -Eq '^kind: (ClusterRole|ClusterRoleBinding|Namespace|PersistentVolume|CustomResourceDefinition)$'; then
  fail "cluster-scoped object rendered in passthrough mode"; fi
echo "$IM" | grep -q '^kind: ClusterRole$' || fail "impersonation must render ClusterRole"
echo "$IM" | grep -A6 '^rules:' | grep -q 'impersonate' || fail "missing impersonate verb"
echo "$IM" | grep -B2 -A8 '^rules:' | grep -Eq 'verbs: \["(get|list|watch|\*)' && fail "SA has read verbs"
# replicaCount > 1 and missing required values must be rejected
helm template t . -f ci/values-passthrough.yaml --set replicaCount=2 >/dev/null 2>&1 && fail "replicaCount=2 accepted"
helm template t . >/dev/null 2>&1 && fail "missing required values accepted"
echo OK

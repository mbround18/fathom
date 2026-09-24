#!/usr/bin/env bash
# Lab only. Switch the kind control plane's kube-apiserver from legacy --oidc-* flags to a
# structured AuthenticationConfiguration accepting the `fathom-desktop` audience.
# Usage: ./apply.sh [node-container]   (default: fathom-oidc-rauthy-control-plane)
set -euo pipefail
N="${1:-fathom-oidc-rauthy-control-plane}"
HERE="$(cd "$(dirname "$0")" && pwd)"
M=/etc/kubernetes/manifests/kube-apiserver.yaml
docker exec "$N" cp -n "$M" /etc/kubernetes/kube-apiserver.yaml.pre-authn-config || true
docker exec -i "$N" sh -c "cat > /etc/kubernetes/pki/fathom-authentication-config.yaml" < "$HERE/authentication-config.yaml"
# Drop the legacy flags (mutually exclusive with --authentication-config), add the new one.
docker exec "$N" sed -i \
  -e '/--oidc-/d' \
  -e '/--authentication-config=/d' \
  -e 's|^\(\s*\)- --authorization-mode=Node,RBAC|&\n\1- --authentication-config=/etc/kubernetes/pki/fathom-authentication-config.yaml|' \
  "$M"
echo "kube-apiserver restarting; wait for readiness: kubectl get --raw /readyz"

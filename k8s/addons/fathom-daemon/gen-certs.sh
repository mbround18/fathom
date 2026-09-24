#!/usr/bin/env bash
# Generate a CA + serving cert for the daemon's aggregated API, store them in Secret fathom-daemon-tls
# (labeled for uninstall), and (with --patch) set the APIService caBundle and disable insecureSkipTLSVerify.
#   gen-certs.sh [--patch]        env: KUBECTL (default "kubectl")
set -euo pipefail
KUBECTL=${KUBECTL:-kubectl}
NS=fathom-daemon; SVC=fathom-daemon
D=$(mktemp -d); trap 'rm -rf "$D"' EXIT
if [ "${1:-}" != "--patch" ]; then
  $KUBECTL create namespace "$NS" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null
  $KUBECTL label namespace "$NS" app.kubernetes.io/name=fathom-daemon --overwrite >/dev/null
  openssl req -x509 -newkey rsa:2048 -nodes -days 825 -subj "/CN=fathom-daemon-ca" -keyout "$D/ca.key" -out "$D/ca.crt" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -subj "/CN=$SVC.$NS.svc" -keyout "$D/tls.key" -out "$D/tls.csr" 2>/dev/null
  printf 'subjectAltName=DNS:%s,DNS:%s.%s,DNS:%s.%s.svc\n' "$SVC" "$SVC" "$NS" "$SVC" "$NS" > "$D/ext"
  openssl x509 -req -in "$D/tls.csr" -CA "$D/ca.crt" -CAkey "$D/ca.key" -CAcreateserial -days 825 -extfile "$D/ext" -out "$D/tls.crt" 2>/dev/null
  $KUBECTL -n "$NS" create secret generic fathom-daemon-tls --from-file=tls.crt="$D/tls.crt" --from-file=tls.key="$D/tls.key" --from-file=ca.crt="$D/ca.crt" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null
  $KUBECTL -n "$NS" label secret fathom-daemon-tls app.kubernetes.io/name=fathom-daemon --overwrite >/dev/null
  echo "secret fathom-daemon-tls created"
else
  CA=$($KUBECTL -n "$NS" get secret fathom-daemon-tls -o jsonpath='{.data.ca\.crt}')
  $KUBECTL patch apiservice v1.daemon.fathom.dev --type=merge -p "{\"spec\":{\"insecureSkipTLSVerify\":false,\"caBundle\":\"$CA\"}}"
fi

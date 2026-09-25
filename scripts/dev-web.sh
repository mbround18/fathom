#!/usr/bin/env bash
# Run Fathom as a website: dev WebSocket bridge (real backend) + Vite UI in `ws` transport.
# Pinned to the local kind cluster; writes only allowed in the sandbox-labelled namespace.
#   scripts/dev-web.sh [--context kind-fathom-dev] [--write-namespace demo]
set -euo pipefail
cd "$(dirname "$0")/.."
CTX="kind-fathom-dev"; WNS="demo"; PORT=18290; UIPORT=5173
while [[ $# -gt 0 ]]; do case "$1" in
  --context) CTX="$2"; shift 2;; --write-namespace) WNS="$2"; shift 2;; *) echo "unknown arg $1" >&2; exit 2;; esac; done
[[ "$CTX" == kind-* ]] || { echo "refusing non-kind context '$CTX'" >&2; exit 2; }
TOKEN="${FATHOM_DEVBRIDGE_TOKEN:-$(head -c16 /dev/urandom | od -An -tx1 | tr -d ' \n')}"
export FATHOM_DEVBRIDGE_TOKEN="$TOKEN"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo build -p fathom-devbridge
target/debug/fathom-devbridge --port "$PORT" --context "$CTX" --write-namespace "$WNS" &
BRIDGE=$!; trap 'kill $BRIDGE 2>/dev/null' EXIT
echo "Open: http://localhost:$UIPORT/?bridge=ws://127.0.0.1:$PORT&token=$TOKEN"
cd ui && VITE_TRANSPORT=ws exec pnpm vite --port "$UIPORT" --host 127.0.0.1

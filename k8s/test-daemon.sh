#!/usr/bin/env bash
# Integration test for the in-cluster daemon (spec 036). Needs a real daemon image (SKIP_BUILD unset builds one).
#   k8s/test-daemon.sh            up-daemon, assert, then down-daemon and assert nothing is left behind
#   KEEP=1 k8s/test-daemon.sh     leave it installed
# Endpoint contract per specs/036-incluster-daemon (contracts/): GET /apis/daemon.fathom.dev/v1/capabilities and
# the NDJSON stream at .../events?after=<seq>&epoch=<id>. Adjust API below if the contract paths differ.
set -euo pipefail
cd "$(dirname "$0")"
CLUSTER="${CLUSTER:-fathom-dev}"
K=(kubectl --context "kind-$CLUSTER")
API=/apis/daemon.fathom.dev/v1
LBL=app.kubernetes.io/name=fathom-daemon
ok() { echo "ok: $*"; }
die() { echo "FAIL: $*" >&2; exit 1; }

./cluster.sh up-daemon
"${K[@]}" -n fathom-daemon wait --for=condition=Available deploy/fathom-daemon --timeout=120s || die "daemon not Ready"
"${K[@]}" wait --for=condition=Available apiservice/v1.daemon.fathom.dev --timeout=60s || die "APIService not Available"
ok "Ready + APIService Available"

caps=$("${K[@]}" get --raw "$API/capabilities") || die "capabilities"
echo "$caps" | jq -e '.protocol and .kinds' >/dev/null || die "capabilities shape: $caps"
ok "capabilities"

# generate events, then read the stream (bounded) and remember the last seq/epoch
"${K[@]}" -n daemon-demo set image deploy/rollout-demo web=nginx:1.27-alpine
sleep 10
stream() { "${K[@]}" get --raw "$API/events$1" --request-timeout=8s 2>/dev/null || true; }
first=$(stream "")
last_seq=$(echo "$first" | jq -s '[.[] | .seq? // empty] | max')
epoch=$(echo "$first" | jq -rs '[.[] | .epoch? // empty][0]')
[ -n "$last_seq" ] && [ "$last_seq" != null ] || die "no sequenced frames in stream"
ok "stream produced frames (seq<=$last_seq)"

# replay after a simulated disconnect: resume from an earlier seq must return the missed frames, in order
mid=$((last_seq / 2))
replay=$(stream "?after=$mid&epoch=$epoch")
echo "$replay" | jq -s -e --argjson m "$mid" '[.[] | .seq? // empty] | (length > 0) and (.[0] > $m)' >/dev/null || die "replay after $mid"
ok "replay after simulated disconnect"

# truncation: a restart starts a new epoch, so the old cursor must yield an explicit truncation marker
"${K[@]}" -n fathom-daemon rollout restart deploy/fathom-daemon
"${K[@]}" -n fathom-daemon rollout status deploy/fathom-daemon --timeout=120s
sleep 3
after=$(stream "?after=$last_seq&epoch=$epoch")
echo "$after" | jq -s -e 'any(.[]; (.type? // .kind? // "") | test("truncat"; "i"))' >/dev/null || die "no truncation marker after restart"
ok "truncation after rollout restart"

[ -n "${KEEP:-}" ] && exit 0
./cluster.sh down-daemon
for i in $(seq 1 30); do
  left=$("${K[@]}" get apiservice,clusterrole,clusterrolebinding,role,rolebinding,namespace -A -l "$LBL" --no-headers 2>/dev/null | wc -l)
  [ "$left" = 0 ] && break; sleep 2
done
[ "$left" = 0 ] || die "$left labeled objects remain after uninstall"
ok "uninstall leaves no labeled objects"

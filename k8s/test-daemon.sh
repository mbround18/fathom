#!/usr/bin/env bash
# Integration test for the in-cluster daemon (spec 036). Needs a real daemon image (SKIP_BUILD unset builds one).
#   k8s/test-daemon.sh            up-daemon, assert, then down-daemon and assert nothing is left behind
#   KEEP=1 k8s/test-daemon.sh     leave it installed
# Endpoint contract per specs/036-incluster-daemon (contracts/): GET /apis/daemon.fathom.dev/v1/v1/capabilities and
# the NDJSON stream at .../v1/stream?after=<seq>&epoch=<id>. Adjust API below if the contract paths differ.
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

caps=$("${K[@]}" get --raw "$API/v1/capabilities") || die "capabilities"
echo "$caps" | jq -e '.protocolMajor == 1 and (.kinds | length > 0) and .perUserFiltering' >/dev/null || die "capabilities shape: $caps"
ok "capabilities"

# generate events, then read the stream (bounded) and remember the last seq/epoch
"${K[@]}" -n daemon-demo rollout restart deploy/rollout-demo   # always changes something, so a re-run still emits events
sleep 10
stream() { "${K[@]}" get --raw "$API/v1/stream$1" --request-timeout=8s 2>/dev/null | jq -Rc 'fromjson? // empty' || true; }
first=$(stream "")
last_seq=$(echo "$first" | jq -s '[.[] | .seq? // empty] | max')
epoch=$(echo "$first" | jq -rs '[.[] | .capabilities.epoch? // empty][0]')
[ -n "$last_seq" ] && [ "$last_seq" != null ] || die "no sequenced frames in stream"
ok "stream produced frames (seq<=$last_seq)"

# replay after a simulated disconnect: resume from an earlier seq must return the missed frames, in order
mid=$((last_seq / 2))
replay=$(stream "?after=$mid&epoch=$epoch")
echo "$replay" | jq -s -e --argjson m "$mid" '[.[] | .seq? // empty] | (length > 0) and (.[0] > $m)' >/dev/null || die "replay after $mid"
ok "replay after simulated disconnect"

# per-user filtering (FR-017): both users may reach the API; only alice may read daemon-demo, bob nothing
cleanup_users() { "${K[@]}" delete clusterrolebinding fathom-it-reader --ignore-not-found >/dev/null 2>&1; "${K[@]}" -n daemon-demo delete rolebinding fathom-it-alice --ignore-not-found >/dev/null 2>&1; }
trap cleanup_users EXIT
"${K[@]}" create clusterrolebinding fathom-it-reader --clusterrole=fathom-daemon-reader --group=system:authenticated >/dev/null
"${K[@]}" -n daemon-demo create rolebinding fathom-it-alice --clusterrole=view --user=alice >/dev/null
sleep 5
"${K[@]}" -n daemon-demo rollout restart deploy/rollout-demo >/dev/null
as_user() { "${K[@]}" --as="$1" --as-group=system:authenticated get --raw "$API/v1/stream" --request-timeout=8s 2>/dev/null | jq -Rc 'fromjson? // empty' || true; }
a=$(as_user alice | jq -s '[.[] | select(.type=="event" or .type=="topologyDelta")] | length')
b=$(as_user bob | jq -s '[.[] | select(.type=="event" or .type=="topologyDelta")] | length')
[ "$a" -gt 0 ] || die "alice (view on daemon-demo) saw no frames"
[ "$b" = 0 ] || die "bob (no access) saw $b frames"
ok "per-user filtering (alice $a frames, bob 0)"

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

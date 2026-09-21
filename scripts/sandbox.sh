#!/usr/bin/env bash
# Manage the fathom-dev sandbox namespace (dev/sandbox/).
#
#   FATHOM_SANDBOX_CONTEXT=<kube-context> scripts/sandbox.sh up|reset|status|down
#
# The context is never inferred from the current kubeconfig context: it must be
# named explicitly. Every mutating subcommand refuses to act on a fathom-dev
# namespace that lacks the fathom.dev/sandbox=true label.
set -euo pipefail

NS=fathom-dev
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../dev/sandbox" && pwd)"
CTX="${FATHOM_SANDBOX_CONTEXT:-}"

die() { echo "sandbox: $*" >&2; exit 1; }
[[ -n "$CTX" ]] || die "set FATHOM_SANDBOX_CONTEXT to the kube context to use (see 'kubectl config get-contexts')"
kc() { kubectl --context "$CTX" "$@"; }

ns_state() {
  if ! kc get ns "$NS" >/dev/null 2>&1; then echo absent; return; fi
  [[ "$(kc get ns "$NS" -o jsonpath='{.metadata.labels.fathom\.dev/sandbox}')" == "true" ]] && echo sandbox || echo foreign
}

require_sandbox() {
  case "$(ns_state)" in
    sandbox) ;;
    absent) die "namespace $NS does not exist in context $CTX (run 'up')" ;;
    foreign) die "namespace $NS in context $CTX is not labeled fathom.dev/sandbox=true; refusing to touch it" ;;
  esac
}

wait_ready() {
  kc -n "$NS" rollout status deploy/web --timeout=180s
  kc -n "$NS" rollout status deploy/chatty-logger --timeout=180s
  kc -n "$NS" rollout status deploy/scale-me --timeout=180s
  kc -n "$NS" rollout status deploy/restart-me --timeout=180s
  kc -n "$NS" rollout status statefulset/kv --timeout=180s
  kc -n "$NS" rollout status daemonset/node-agent --timeout=180s
}

case "${1:-}" in
  up)
    [[ "$(ns_state)" == foreign ]] && die "namespace $NS exists in context $CTX without the sandbox label; refusing to adopt it"
    kc apply -k "$DIR"
    wait_ready
    ;;
  reset)
    # Back to the committed baseline: drop e2e throwaways, re-apply, restore replica counts.
    require_sandbox
    kc -n "$NS" delete pods,deployments,configmaps -l fathom.dev/e2e=true --ignore-not-found --wait=false
    kc apply -k "$DIR"
    kc -n "$NS" scale deploy/scale-me --replicas=1
    wait_ready
    ;;
  status)
    require_sandbox
    kc -n "$NS" get deploy,statefulset,daemonset,job,cronjob,pods -o wide
    kc -n "$NS" describe resourcequota sandbox-quota | sed -n '/Resource/,$p'
    ;;
  down)
    require_sandbox
    kc delete ns "$NS" --wait=true
    ;;
  *)
    die "usage: FATHOM_SANDBOX_CONTEXT=<ctx> $0 up|reset|status|down"
    ;;
esac

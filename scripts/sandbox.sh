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

# The Helm panes show a revision history and a ROLL BACK control, and the promo shoot photographs
# them — but Flux installs podinfo at revision 1, where there is nothing to roll back to. Bump one
# cosmetic chart value once to produce a second revision. Idempotent: a release that already has
# a history is left alone, and a cluster without Flux is simply skipped.
second_helm_revision() {
  local deadline
  kc -n "$NS" wait --for=condition=Ready helmrelease/podinfo --timeout=300s || {
    echo "sandbox: podinfo HelmRelease not Ready; skipping the second-revision bump" >&2; return 0
  }
  revs() { kc -n "$NS" get secret -l owner=helm,name=podinfo -o name 2>/dev/null | wc -l; }
  if [[ "$(revs)" -ge 2 ]]; then return 0; fi
  echo "sandbox: upgrading podinfo once so the Helm panes have a revision history"
  kc -n "$NS" patch helmrelease podinfo --type merge \
    -p '{"spec":{"values":{"ui":{"message":"Fathom sandbox"}}}}'
  deadline=$((SECONDS + 300))
  until [[ "$(revs)" -ge 2 ]]; do
    [[ $SECONDS -lt $deadline ]] || die "podinfo did not reach a second Helm revision within 300s"
    sleep 5
  done
}

# The live Helm suite (ui/tests/live/qs-006-helm-mutate.spec.ts) mutates a release that Flux does
# not own, so it needs a bare `helm install` of the same chart with a history to roll back through.
# Hand-installing it is how the suite ends up failing on a freshly built cluster. Idempotent, and
# skipped with a warning when helm is not on PATH.
BARE_RELEASE=quickstart-bare
BARE_CHART="oci://ghcr.io/stefanprodan/charts/podinfo"
BARE_VERSION=6.15.0
bare_helm_release() {
  command -v helm >/dev/null 2>&1 || {
    echo "sandbox: helm not on PATH; skipping the $BARE_RELEASE fixture" >&2; return 0
  }
  local h=(helm --kube-context "$CTX" -n "$NS")
  # -o json is one line, so count the occurrences rather than the matching lines.
  if [[ "$("${h[@]}" history "$BARE_RELEASE" -o json 2>/dev/null | grep -o '"revision"' | wc -l)" -ge 2 ]]; then
    return 0
  fi
  echo "sandbox: installing $BARE_RELEASE with a two-revision history for the live Helm suite"
  "${h[@]}" upgrade --install "$BARE_RELEASE" "$BARE_CHART" --version "$BARE_VERSION" --wait
  "${h[@]}" upgrade "$BARE_RELEASE" "$BARE_CHART" --version "$BARE_VERSION" \
    --set ui.message="bare release" --wait
}

case "${1:-}" in
  up)
    [[ "$(ns_state)" == foreign ]] && die "namespace $NS exists in context $CTX without the sandbox label; refusing to adopt it"
    kc apply -k "$DIR"
    wait_ready
    second_helm_revision
    bare_helm_release
    ;;
  reset)
    # Back to the committed baseline: drop e2e throwaways, re-apply, restore replica counts.
    require_sandbox
    kc -n "$NS" delete pods,deployments,configmaps -l fathom.dev/e2e=true --ignore-not-found --wait=false
    kc apply -k "$DIR"
    kc -n "$NS" scale deploy/scale-me --replicas=1
    wait_ready
    second_helm_revision
    bare_helm_release
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

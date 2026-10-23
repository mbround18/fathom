#!/usr/bin/env bash
# Local resource hygiene: find (and optionally clean) what agent runs leave behind, so they don't
# eat the disk and RAM: Claude worktrees, kind clusters, Docker build cache and dangling images,
# and Depths test leftovers (depths-test-* compose projects, temp stacks roots).
#
#   scripts/hygiene.sh [check]   report only; exits 1 when anything dangling is found
#   scripts/hygiene.sh clean     remove only what is provably safe to remove (see below)
#
# Env:
#   FATHOM_KEEP_CLUSTERS  space-separated kind clusters that are never flagged (default: fathom-dev)
#   CACHE_AGE             build cache older than this is pruned by `clean` (default: 24h)
#   CACHE_MAX_GB          `check` flags reclaimable build cache above this (default: 100)
#
# Safety rules (clean):
#   - a worktree locked by a live Claude session (pid in the lock reason) is never touched
#   - a worktree is removed only if its branch is merged into main AND it has no local changes;
#     unmerged or dirty worktrees are reported for a human to salvage, never deleted
#   - kind clusters: only fathom-* clusters outside FATHOM_KEEP_CLUSTERS, and only while no agent
#     is live (an agent may own one); other clusters are reported, never deleted
#   - Docker: only build cache older than CACHE_AGE and dangling images; containers, volumes and
#     tagged images (other projects share this daemon) are never touched
#   - Depths tests: compose projects named depths-test-* are only reported (their volumes may be
#     wanted for a failing test); temp stacks roots ($TMPDIR/depths-test-*) older than an hour,
#     so never a running test's, are removed
set -euo pipefail

CMD="${1:-check}"
case "$CMD" in check|clean) ;; *) echo "usage: $0 [check|clean]" >&2; exit 2 ;; esac
KEEP=" ${FATHOM_KEEP_CLUSTERS:-fathom-dev} "
CACHE_AGE="${CACHE_AGE:-24h}"
CACHE_MAX_GB="${CACHE_MAX_GB:-100}"

ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$ROOT"
WT_DIR="$ROOT/.claude/worktrees"
dirty=0
live_agents=0
flag() { echo "  ! $*"; dirty=1; }

echo "== Claude worktrees"
# Porcelain records are blank-line separated: worktree / HEAD / branch / [locked <reason>].
while IFS=$'\t' read -r path branch lock; do
  [[ "$path" == "$WT_DIR"/* ]] || continue
  name="${path#"$WT_DIR"/}"
  pid=""
  [[ "$lock" =~ \(pid\ ([0-9]+) ]] && pid="${BASH_REMATCH[1]}"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    echo "  active  $name (locked by live Claude session, pid $pid)"
    live_agents=$((live_agents + 1))
    continue
  fi
  changes=$(git -C "$path" status --porcelain 2>/dev/null | wc -l)
  if [[ -n "$branch" ]] && git merge-base --is-ancestor "$branch" main 2>/dev/null && [[ "$changes" -eq 0 ]]; then
    flag "merged  $name ($branch, clean)"
    if [[ "$CMD" == clean ]]; then
      [[ -n "$lock" ]] && git worktree unlock "$path"
      git worktree remove "$path" && git branch -d "$branch" >/dev/null && echo "    removed"
    fi
  else
    flag "review  $name (${branch:-detached}; $(git rev-list --count "main..${branch:-HEAD}" 2>/dev/null || echo '?') unmerged commits, $changes local changes): salvage by hand"
  fi
done < <(git worktree list --porcelain | awk '
  /^worktree /{p=substr($0,10); b=""; l=""}
  /^branch /{b=substr($0,19)}
  /^locked/{l=substr($0,8); if (l=="") l="locked"}
  /^$/{print p "\t" b "\t" l}
  END{if (p!="") print p "\t" b "\t" l}' | sort -u)
[[ "$CMD" == clean ]] && git worktree prune

echo "== kind clusters (keep:${KEEP%% })"
if command -v kind >/dev/null; then
  for c in $(kind get clusters 2>/dev/null); do
    if [[ "$KEEP" == *" $c "* ]]; then echo "  keep    $c"; continue; fi
    flag "extra   $c"
    if [[ "$CMD" == clean ]]; then
      if [[ "$c" != fathom-* ]]; then echo "    not a fathom-* cluster: left alone"
      elif (( live_agents > 0 )); then echo "    $live_agents agent(s) live and may own it: left alone"
      else kind delete cluster --name "$c" >/dev/null 2>&1 && echo "    deleted"; fi
    fi
  done
fi

echo "== Docker"
if docker info >/dev/null 2>&1; then
  cache=$(docker system df --format '{{.Type}}|{{.Reclaimable}}' | awk -F'|' '$1=="Build Cache"{print $2}')
  gb=$(awk -v s="${cache%% *}" 'BEGIN{
    n=s+0; u=s; gsub(/[0-9.]/,"",u)
    printf "%d", (u=="TB"?n*1000:u=="GB"?n:u=="MB"?n/1000:0)}')
  if (( gb > CACHE_MAX_GB )); then flag "build cache reclaimable: $cache (> ${CACHE_MAX_GB}GB)"
  else echo "  build cache reclaimable: ${cache:-0B}"; fi
  dangling=$(docker images -qf dangling=true | wc -l)
  (( dangling > 0 )) && flag "$dangling dangling image(s)"
  if [[ "$CMD" == clean ]]; then
    docker builder prune -f --filter "until=$CACHE_AGE" | tail -1 | sed 's/^/    cache: /'
    docker image prune -f | tail -1 | sed 's/^/    images: /'
  fi
fi

echo "== Depths test leftovers"
if docker info >/dev/null 2>&1; then
  while read -r proj; do
    [[ -n "$proj" ]] && flag "review  compose project $proj: docker compose -p $proj down --volumes"
  done < <(docker compose ls --all --format json 2>/dev/null \
    | python3 -c 'import json,sys; [print(p["Name"]) for p in json.load(sys.stdin) if p["Name"].startswith("depths-test-")]' 2>/dev/null || true)
fi
while read -r d; do
  [[ -n "$d" ]] || continue
  flag "stale   temp stacks root $d"
  [[ "$CMD" == clean ]] && rm -rf -- "$d" && echo "    removed"
done < <(find "${TMPDIR:-/tmp}" -maxdepth 1 -name 'depths-test-*' -mmin +60 2>/dev/null)

echo "== cargo target (report only)"
du -sh "$ROOT/target" 2>/dev/null | sed 's/^/  /' || true

if [[ "$CMD" == check && $dirty -eq 1 ]]; then
  echo "dangling resources found: run scripts/hygiene.sh clean (items marked 'review' need a human)"
  exit 1
fi

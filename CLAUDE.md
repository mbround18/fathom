# Fathom: agent working rules

## Clean up after yourself: resource hygiene

Agent runs (worktrees, kind clusters, image builds) have filled this machine before. Treat
cleanup as part of finishing a task, not as optional housekeeping.

- **Worktrees**: after merging an agent's branch, remove its worktree and delete the branch in the
  same step. Before removing an unmerged or dirty worktree, diff it and salvage anything worth
  keeping into main.
- **kind clusters**: an agent that creates a cluster (`fathom-hosted-*`, `fathom-daemon-*`,
  `fathom-fed-*`, `fathom-prov-*`, ...) deletes it before reporting, unless it says in its report
  why the cluster must stay. `fathom-dev` is the long-lived dev cluster; keep it.
- **Docker**: prune build cache after a batch of image builds (`docker builder prune --filter
  until=24h`). Never touch containers, volumes or tagged images: other projects share this daemon.
- **Check**: run `scripts/hygiene.sh` before reporting a batch done, and after merging agent work.
  It exits 1 when something dangles. `scripts/hygiene.sh clean` removes only what is provably
  safe; items marked `review` need a human (or you) to salvage first.

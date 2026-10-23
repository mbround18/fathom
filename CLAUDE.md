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
- **Parallel agents share one machine** (32 cores, 31GB RAM). A full Playwright suite at default
  parallelism uses ~12GB of browsers, so two at once run the box out of memory. `playwright.config.ts`
  now sizes its own workers from the current load average and `MemAvailable`
  (`ui/tests/support/workers.ts`) and prints what it picked, so the main e2e suite no longer needs
  `--workers=<n>`; override with `PLAYWRIGHT_WORKERS` when you need to pin it. Cargo is not
  auto-sized: still use `CARGO_BUILD_JOBS=4` when other agents are active.

## One CI at a time

Three `scripts/ci.sh` runs at once drove load past 200, restarted the Dagger engine and made the
Playwright suite fail on timeouts. So:

- **Subagents never run `scripts/ci.sh`, `paws` or `dagger`.** They gate on targeted tests for what
  they touched (`cargo test -p <crate>`, `pnpm --filter <pkg> test`, one Playwright spec file), one
  command at a time, with `CARGO_BUILD_JOBS=4`. Say so in every fan-out brief.
- **The main session runs the full gate once**, on the combined branch after merging agent work,
  never per agent branch.
- `scripts/ci.sh` takes a machine-wide lock (`$XDG_RUNTIME_DIR/fathom-ci.lock`, holder in
  `fathom-ci.lock.owner`): a second run waits, or exits 75 with `CI_LOCK_WAIT=0`. Don't work
  around it. If a run is stale, stop the session that owns it, not the lock.
- Don't run the full Playwright suite alongside a CI run, or two suites together.

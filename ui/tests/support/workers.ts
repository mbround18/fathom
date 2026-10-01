/**
 * Worker sizing for the local Playwright suites.
 *
 * Playwright's default is half the cores, which on this machine is 16 browsers — enough to drive
 * the load average past 39 and make the desktop unusable while a suite runs (CLAUDE.md: parallel
 * agents share one 32-core / 31 GB box). The default is also blind to whatever else is already
 * running: another agent's suite, a cargo build, a kind cluster.
 *
 * So measure instead of guessing. The budget is the smaller of what the CPU and the RAM can
 * currently afford, and the suite never claims more than half the machine.
 */
import { availableParallelism, freemem, loadavg, totalmem } from 'node:os'
import { readFileSync } from 'node:fs'

const GB = 1024 ** 3

/** Peak RSS of one worker's browser plus its page, rounded up from ~1.1 GB measured on WebKit. */
const PER_WORKER_GB = 1.5

/** Left for the desktop session, the dev cluster and whatever else is running. */
const RESERVED_GB = 6

/**
 * `MemAvailable` (what the kernel thinks is obtainable without swapping) rather than `freemem()`,
 * which ignores reclaimable page cache and so reads far too low on a machine that has been busy.
 */
function availableGb(): number {
  try {
    const line = readFileSync('/proc/meminfo', 'utf8').match(/^MemAvailable:\s+(\d+) kB$/m)
    if (line?.[1]) return Number(line[1]) / 1024 / 1024
  } catch {
    // Not Linux, or /proc unreadable — fall through.
  }
  return freemem() / GB
}

/**
 * How many Playwright workers this machine can take *right now*.
 *
 * `PLAYWRIGHT_WORKERS` overrides it outright. Under CI the runner is dedicated and its own sizing
 * wins, so this returns undefined and Playwright keeps its default.
 */
export function autoWorkers(): number | undefined {
  if (process.env.PLAYWRIGHT_WORKERS) {
    const n = Number(process.env.PLAYWRIGHT_WORKERS)
    if (Number.isFinite(n) && n >= 1) return Math.floor(n)
  }
  if (process.env.CI) return undefined

  const cores = availableParallelism()
  // The 1-minute load average counts work already queued, so idle cores is what is left over for
  // us. Half of that: a browser worker is not the only thing a test needs a core for.
  const idle = Math.max(1, cores - Math.round(loadavg()[0] ?? 0))
  const byCpu = Math.max(1, Math.floor(idle / 2))
  const byMem = Math.max(1, Math.floor((availableGb() - RESERVED_GB) / PER_WORKER_GB))
  const workers = Math.max(1, Math.min(byCpu, byMem, Math.floor(cores / 2)))

  if (!process.env.PLAYWRIGHT_QUIET_WORKERS) {
    const mem = availableGb().toFixed(1)
    const total = (totalmem() / GB).toFixed(0)
    process.stderr.write(
      `[workers] ${String(workers)} (cores ${String(cores)}, load ${(loadavg()[0] ?? 0).toFixed(1)}, ` +
        `mem ${mem}/${total} GB free) — override with PLAYWRIGHT_WORKERS\n`,
    )
  }
  return workers
}

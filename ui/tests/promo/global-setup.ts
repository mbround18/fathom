/**
 * Promo global setup: start from an empty shots directory, then bring up the read-only bridge.
 *
 * `promo-out/shots/` used to survive between runs and the composer only checked that each PNG
 * existed, so a scene that failed shipped the *previous* run's photograph of the same name into
 * the PDF. The brochure looked complete; that is how a Helm inspector showing nothing but loading
 * skeletons reached a 41-page PDF unnoticed. Clearing the shots first makes a failed scene a
 * missing file, and `compose.spec.ts` now refuses to build the PDF when one is missing.
 *
 * A subset run (`--grep`, or a test-file argument) is a deliberate partial re-shoot, so it
 * keeps the shots it is not re-taking. `compose.spec.ts` is filtered out of such a run and so
 * never builds a PDF from them.
 */
import { mkdirSync, rmSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { startBridge } from '../live/global-setup'

const SHOTS_DIR = fileURLToPath(new URL('../../promo-out/shots/', import.meta.url))

/**
 * Whether this run is a deliberate subset rather than the whole shoot.
 *
 * `FullConfig.grep` is not it: a `--grep` on the command line never reaches it (it still matches
 * everything there), so the argv Playwright was started with is the only honest source.
 */
const SUBSET_FLAGS = new Set([
  '-g',
  '--grep',
  '--grep-invert',
  '--last-failed',
  '--only-changed',
  '--shard',
])

/** Flags that take a separate value, so the value is not mistaken for a file filter. */
const VALUE_FLAGS = new Set([
  '-c',
  '--config',
  '-j',
  '--workers',
  '--project',
  '--reporter',
  '--retries',
  '--timeout',
  '--output',
  '--repeat-each',
  '--max-failures',
  ...SUBSET_FLAGS,
])

function isSubsetRun(): boolean {
  const args = process.argv.slice(process.argv.indexOf('test') + 1)
  for (let i = 0; i < args.length; i++) {
    const arg = args[i] ?? ''
    const [flag] = arg.split('=')
    if (SUBSET_FLAGS.has(flag ?? '')) return true
    if (arg.startsWith('-')) {
      if (VALUE_FLAGS.has(arg)) i++
      continue
    }
    // A bare positional is a test-file filter, so this run is a subset too.
    return true
  }
  return false
}

export default async function globalSetup(): Promise<void> {
  if (isSubsetRun()) {
    console.log('[promo] subset run: keeping existing shots (no PDF is built by a partial run)')
  } else {
    rmSync(SHOTS_DIR, { recursive: true, force: true })
    mkdirSync(SHOTS_DIR, { recursive: true })
    console.log(`[promo] cleared ${SHOTS_DIR} — every shot in the PDF comes from this run`)
  }
  await startBridge()
}

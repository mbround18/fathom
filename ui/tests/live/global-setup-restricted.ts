/**
 * Global setup for the restricted-user walk (playwright.restricted.config.ts). Starts the live
 * bridge, read-only, as a namespace-scoped identity:
 *
 *   FATHOM_RESTRICTED_KUBECONFIG=<restricted kubeconfig> FATHOM_LIVE_CONTEXT=fathom-restricted
 *
 * The kubeconfig becomes the only KUBECONFIG for the bridge and kubectl. The sandbox label check
 * still works: a RoleBinding to `view`/`edit` lets the user GET its own namespace object. Without
 * FATHOM_RESTRICTED_KUBECONFIG nothing starts and the walk skips.
 */
import { execFileSync } from 'node:child_process'

import { RESTRICTED_ENV, liveContext, restrictedKubeconfig } from './env'
import { startBridge } from './global-setup'

/** The identity must NOT be able to list namespaces cluster-wide, or this walk proves nothing. */
function checkRestricted(context: string): void {
  let answer: string
  try {
    answer = execFileSync(
      'kubectl',
      ['--context', context, 'auth', 'can-i', 'list', 'namespaces', '--all-namespaces'],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
    ).trim()
  } catch (err) {
    // `can-i` exits 1 for "no".
    answer = ((err as { stdout?: string }).stdout ?? '').trim()
  }
  if (answer !== 'no') {
    throw new Error(
      `\n\n[restricted walk] context '${context}' can list namespaces cluster-wide ` +
        `(can-i: '${answer}'); point FATHOM_LIVE_CONTEXT at the namespace-scoped identity.\n`,
    )
  }
}

export default async function globalSetup(): Promise<void> {
  const kubeconfig = restrictedKubeconfig()
  if (!kubeconfig) {
    console.log(`[restricted walk] ${RESTRICTED_ENV.kubeconfig} not set: skipping`)
    return
  }
  process.env.KUBECONFIG = kubeconfig
  checkRestricted(liveContext())
  await startBridge(['--read-only'], 'STRICT: read-only, restricted identity')
}

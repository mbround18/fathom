/** Names shared by the live suite's config, global setup/teardown, and specs. */
import { existsSync } from 'node:fs'

/** The only namespace the live suite reads from or writes to. */
export const LIVE_NAMESPACE = 'fathom-dev'
export const SANDBOX_LABEL = ['fathom.dev/sandbox', 'true'] as const
/** Every object a live spec creates carries this label; `restoreBaseline` deletes them. */
export const E2E_LABEL = ['fathom.dev/e2e', 'true'] as const

/**
 * Port of the `--mode live` UI preview (default 4174). `FATHOM_LIVE_UI_PORT` moves it, so two
 * worktrees can run the live or promo suite at the same time.
 */
export const APP_PORT = Number(process.env.FATHOM_LIVE_UI_PORT ?? 4174)
export const APP_ORIGIN = `http://localhost:${String(APP_PORT)}`
/** Build and serve the live bundle on `APP_PORT` (playwright webServer command). */
export const LIVE_SERVE_COMMAND =
  'pnpm exec vite build --mode live --outDir dist-live && ' +
  `pnpm exec vite preview --outDir dist-live --port ${String(APP_PORT)} --strictPort`

/** Process env the global setup fills in for the workers (never written to disk). */
export const ENV = {
  context: 'FATHOM_LIVE_CONTEXT',
  /** Optional input: fixed bridge port (default: a free one). */
  port: 'FATHOM_LIVE_BRIDGE_PORT',
  token: 'FATHOM_LIVE_BRIDGE_TOKEN',
  pid: 'FATHOM_LIVE_BRIDGE_PID',
} as const

/** `$FATHOM_LIVE_CONTEXT`, required and never defaulted (not even to the current context). */
export function liveContext(): string {
  const context = process.env[ENV.context]?.trim()
  if (!context) {
    throw new Error(
      '\n\n[live e2e] FATHOM_LIVE_CONTEXT is not set.\n\n' +
        'The live suite runs the real UI against a real cluster, so the kube context must be ' +
        'named explicitly (it is never taken from your current kubeconfig context):\n\n' +
        '    FATHOM_LIVE_CONTEXT=<context> pnpm -C ui e2e:live\n\n' +
        "List contexts with 'kubectl config get-contexts'. The context needs the fathom-dev " +
        'sandbox: FATHOM_SANDBOX_CONTEXT=<context> scripts/sandbox.sh up\n',
    )
  }
  return context
}

/**
 * Restricted-user walk (tests/live/restricted-walk.spec.ts): path of a kubeconfig for a
 * namespace-scoped identity (k8s/samples/restricted-user.yaml). Unset or missing file: skipped.
 */
export const RESTRICTED_ENV = { kubeconfig: 'FATHOM_RESTRICTED_KUBECONFIG' } as const

/** The restricted walk's kubeconfig, or undefined when the walk is not enabled. */
export function restrictedKubeconfig(): string | undefined {
  const path = process.env[RESTRICTED_ENV.kubeconfig]?.trim()
  return path && existsSync(path) ? path : undefined
}

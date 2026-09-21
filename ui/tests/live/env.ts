/** Names shared by the live suite's config, global setup/teardown, and specs. */

/** The only namespace the live suite reads from or writes to. */
export const LIVE_NAMESPACE = 'fathom-dev'
export const SANDBOX_LABEL = ['fathom.dev/sandbox', 'true'] as const
/** Every object a live spec creates carries this label; `restoreBaseline` deletes them. */
export const E2E_LABEL = ['fathom.dev/e2e', 'true'] as const

export const APP_ORIGIN = 'http://localhost:4174'

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

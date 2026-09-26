/**
 * Global setup for the opt-in prod RBAC specs (playwright.prod-rbac.config.ts). Starts the same
 * devbridge as the live suite, but in STRICT mode:
 *
 *   setup  (FATHOM_PROD_RBAC_SETUP=1):  `--allow-create` for exactly the four planned bindings;
 *                                       every other cluster write is refused by the bridge.
 *   verify (FATHOM_PROD_RBAC_VERIFY=1): `--read-only`; no cluster write passes at all.
 *
 * Refuses to start without one of the two, so the prod config can never run by accident.
 */
import { startBridge } from './global-setup'
import { BINDINGS, allowCreateSpec, prodRbacMode } from './rbac-prod-plan'

export default async function globalSetup(): Promise<void> {
  const mode = prodRbacMode()
  if (!mode) {
    throw new Error(
      '\n\n[prod rbac] refusing to start: set FATHOM_PROD_RBAC_SETUP=1 (create the planned ' +
        'bindings) or FATHOM_PROD_RBAC_VERIFY=1 (read-only checks as the agent identity).\n',
    )
  }
  if (mode === 'setup') {
    const specs = BINDINGS.map(allowCreateSpec)
    await startBridge(
      specs.flatMap((s) => ['--allow-create', s]),
      `STRICT: create-only [${specs.join(', ')}]`,
    )
  } else {
    await startBridge(['--read-only'], 'STRICT: read-only')
  }
}

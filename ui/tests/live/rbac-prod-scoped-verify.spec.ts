/**
 * OPT-IN, PROD, READ-ONLY: connect Fathom AS the scoped agent (an operator-minted kubeconfig for
 * `system:serviceaccount:fathom-e2e:fathom-agent`) and check that the RBAC-aware UI enables
 * mutation controls only in fathom-dev/fathom-e2e. Runs only via playwright.prod-rbac.config.ts
 * with `FATHOM_PROD_RBAC_VERIFY=1`; the bridge is `--read-only`, and no control is clicked.
 *
 *   FATHOM_PROD_RBAC_VERIFY=1 KUBECONFIG=~/.kube/fathom-agent.yaml \
 *     FATHOM_LIVE_CONTEXT=prod-fathom-agent pnpm -C ui e2e:prod-rbac
 */
import type { Page } from '@playwright/test'

import { expect, navigateTo, tapBridge, test } from './helpers'
import { AGENT_NAMESPACES, canI, prodRbacMode } from './rbac-prod-plan'

test.skip(prodRbacMode() !== 'verify', 'opt-in: set FATHOM_PROD_RBAC_VERIFY=1 (agent identity)')

const OUTSIDE = ['default', 'kube-system'] as const

/** `data-capability` of the gated create-binding launcher in the current pane. */
function launcher(page: Page) {
  return page.getByTestId('rbac-binding-create-open')
}

test('kubectl as the agent: can-i matrix (read-only SelfSubjectAccessReviews)', () => {
  for (const ns of AGENT_NAMESPACES) {
    expect(canI('create', 'deployments', { namespace: ns }, false), `create deploy in ${ns}`).toBe(
      true,
    )
    expect(canI('create', 'rolebindings', { namespace: ns }, false), `create rb in ${ns}`).toBe(
      true,
    )
  }
  for (const ns of OUTSIDE) {
    expect(canI('create', 'deployments', { namespace: ns }, false), `create deploy in ${ns}`).toBe(
      false,
    )
    expect(canI('delete', 'pods', { namespace: ns }, false), `delete pods in ${ns}`).toBe(false)
    expect(canI('get', 'secrets', { namespace: ns }, false), `get secrets in ${ns}`).toBe(false)
  }
  expect(canI('list', 'pods', { allNamespaces: true }, false), 'list pods -A').toBe(true)
  expect(canI('list', 'nodes', {}, false), 'list nodes').toBe(true)
  expect(canI('create', 'clusterrolebindings', {}, false), 'create CRB').toBe(false)
})

test('RBAC-aware UI: create-binding enabled only in fathom-dev / fathom-e2e', async ({
  page,
  app,
}) => {
  const tap = await tapBridge(page)
  await app.open()
  await app.selectContext()

  const seen: Record<string, string | null> = {}
  for (const ns of [...AGENT_NAMESPACES, ...OUTSIDE]) {
    await app.selectNamespace(ns)
    await navigateTo(page, 'Role Bindings', 'Access Control')
    const expected = (AGENT_NAMESPACES as readonly string[]).includes(ns) ? 'allowed' : 'denied'
    await expect(launcher(page), `New RoleBinding in ${ns}`).toHaveAttribute(
      'data-capability',
      expected,
    )
    seen[`RoleBinding@${ns}`] = await launcher(page).getAttribute('data-capability')
    if (expected === 'denied') await expect(launcher(page)).toBeDisabled()
    else await expect(launcher(page)).toBeEnabled()
    await page.screenshot({ path: test.info().outputPath(`rolebindings-${ns}.png`) })
  }

  await navigateTo(page, 'Cluster Role Bindings', 'Access Control')
  await expect(launcher(page), 'New ClusterRoleBinding').toHaveAttribute(
    'data-capability',
    'denied',
  )
  await expect(launcher(page)).toBeDisabled()
  seen.ClusterRoleBinding = await launcher(page).getAttribute('data-capability')
  await page.screenshot({ path: test.info().outputPath('clusterrolebindings.png') })

  test.info().annotations.push({ type: 'capabilities', description: JSON.stringify(seen) })
  expect(tap.writes(), 'nothing is written in verify mode').toEqual([])
})

/**
 * OPT-IN, PROD: create the scoped RBAC for `system:serviceaccount:fathom-e2e:fathom-agent`
 * through Fathom's own 011 RBAC UI (create form -> diff preview -> confirm), then check it with
 * the namespace-access lookup (011 US4). Runs only via playwright.prod-rbac.config.ts with
 * `FATHOM_PROD_RBAC_SETUP=1`; skipped otherwise.
 *
 * Idempotent: an existing binding is verified against the plan (roleRef, sole subject, label)
 * and never re-created or modified. Nothing here writes to the cluster except the bridge, whose
 * strict `--allow-create` list holds exactly the four planned bindings (global-setup-prod-rbac);
 * the spec also asserts every write it sent was one of them. kubectl is used read-only.
 *
 * Gap: 011 has no Role/ClusterRole create flow (it only reads them, spec FR list), so ClusterRole
 * `fathom-agent-cluster-read` must already exist: create it from
 * k8s/prod-scoped-agent/rbac.yaml. The first test fails with that instruction if it is missing.
 */
import type { Page } from '@playwright/test'

import type { BackendCmd } from '../../src/protocol/types'

import { expect, navigateTo, tapBridge, test, type BridgeTap } from './helpers'
import {
  AGENT,
  AGENT_NAMESPACES,
  BINDINGS,
  CLUSTER_ROLE,
  MANAGED_BY,
  bindingMismatches,
  getRbac,
  prodRbacMode,
  type PlannedBinding,
} from './rbac-prod-plan'

test.skip(prodRbacMode() !== 'setup', 'opt-in: set FATHOM_PROD_RBAC_SETUP=1 (prod RBAC setup)')
test.describe.configure({ mode: 'serial' })

const CLUSTER_ROLE_RULES: readonly [string, readonly string[]][] = [
  ['', ['nodes', 'namespaces', 'persistentvolumes']],
  ['storage.k8s.io', ['storageclasses']],
  ['apiextensions.k8s.io', ['customresourcedefinitions']],
  ['apiregistration.k8s.io', ['apiservices']],
  ['rbac.authorization.k8s.io', ['clusterroles', 'clusterrolebindings', 'roles', 'rolebindings']],
]

function label(b: PlannedBinding): string {
  return b.namespace ? `${b.kind} ${b.namespace}/${b.name}` : `${b.kind} ${b.name}`
}

/** Every write the page sent must be the create of exactly `b`; the bridge refused nothing. */
function expectOnlyCreateOf(tap: BridgeTap, b: PlannedBinding): void {
  const writes = tap.writes()
  expect(writes.length, 'exactly one cluster write').toBe(1)
  const cmd = writes[0] as Extract<BackendCmd, { type: 'CreateResourceYaml' }>
  expect(cmd.type).toBe('CreateResourceYaml')
  expect(cmd.data).toMatchObject({
    api_version: 'rbac.authorization.k8s.io/v1',
    kind: b.kind,
    namespace: b.namespace,
    name: b.name,
  })
  expect(tap.errors, 'bridge refusals').toEqual([])
}

async function createThroughUi(page: Page, tap: BridgeTap, b: PlannedBinding): Promise<void> {
  const isRb = b.kind === 'RoleBinding'
  // The "New RoleBinding" launcher is gated on the active namespace (selected by the caller).
  await navigateTo(page, isRb ? 'Role Bindings' : 'Cluster Role Bindings', 'Access Control')
  const launcher = page.getByTestId('rbac-binding-create-open')
  // No extra timeout: `Initialize` no longer holds the backend's state lock through optional
  // metrics/Prometheus discovery (it runs in `post_connect` after the lock is released), so
  // `RefreshRoleBindings` is not queued behind a cold cluster's discovery any more (the "Loading
  // Role Bindings hang" in checklists/prod-scoped-agent.md).
  await expect(launcher).toHaveAttribute('data-capability', 'allowed')
  await launcher.click()

  const form = page.getByTestId('rbac-binding-form')
  await expect(form).toBeVisible()
  await form.getByTestId('rbac-binding-name').fill(b.name)
  if (isRb && b.namespace) {
    await form.getByTestId('rbac-binding-namespace').fill(b.namespace)
    await form.getByTestId('rbac-binding-roleref-kind').selectOption('ClusterRole')
  }
  await form.getByTestId('rbac-binding-roleref-name').fill(b.roleRef)
  await form.getByTestId('rbac-subject-add').click()
  await form.getByTestId('rbac-subject-kind').selectOption(AGENT.kind)
  await form.getByTestId('rbac-subject-name').fill(AGENT.name)
  await form.getByTestId('rbac-subject-namespace').fill(AGENT.namespace)
  await form.getByTestId('rbac-binding-labels').fill(MANAGED_BY.join('='))

  const review = form.getByTestId('rbac-binding-review')
  await expect(review).toBeEnabled()
  await review.click()

  // Diff preview: the full proposed object, nothing else.
  const diff = form.getByTestId('yaml-diff')
  await expect(diff).toBeVisible()
  for (const text of [
    `kind: ${b.kind}`,
    `name: ${b.name}`,
    `name: ${b.roleRef}`,
    'kind: ClusterRole',
    `name: ${AGENT.name}`,
    `namespace: ${AGENT.namespace}`,
    `${MANAGED_BY[0]}: ${MANAGED_BY[1]}`,
  ]) {
    await expect(diff).toContainText(text)
  }
  if (b.namespace) await expect(diff).toContainText(`namespace: ${b.namespace}`)
  await page.screenshot({
    path: test.info().outputPath(`diff-${b.kind}-${b.namespace ?? 'cluster'}.png`),
  })

  expect(tap.writes(), 'no write before confirmation').toEqual([])
  await form.getByTestId('yaml-apply').click()
  const dialog = page.getByTestId('confirm-dialog')
  await expect(dialog).toBeVisible()
  await expect(dialog).toContainText(b.name)
  await dialog.getByTestId('confirm-submit').click()

  await expect(form.getByTestId('rbac-binding-applied')).toBeVisible({ timeout: 30_000 })
  expectOnlyCreateOf(tap, b)
}

test('ClusterRole fathom-agent-cluster-read exists (created outside the UI: 011 gap)', () => {
  const role = getRbac('ClusterRole', CLUSTER_ROLE, null)
  expect(
    role,
    `ClusterRole ${CLUSTER_ROLE} is missing. 011 has no Role/ClusterRole create flow; create it ` +
      'with: kubectl --context <ctx> create -f <(yq "select(.kind == \\"ClusterRole\\")" ' +
      'k8s/prod-scoped-agent/rbac.yaml)',
  ).not.toBeNull()
  if (!role) return
  expect(role.metadata.labels?.[MANAGED_BY[0]]).toBe(MANAGED_BY[1])
  for (const [group, resources] of CLUSTER_ROLE_RULES) {
    for (const resource of resources) {
      const covered = (role.rules ?? []).some(
        (r) =>
          (r.apiGroups ?? []).includes(group) &&
          (r.resources ?? []).includes(resource) &&
          ['get', 'list', 'watch'].every((v) => (r.verbs ?? []).includes(v)),
      )
      expect(covered, `${resource}.${group || 'core'} get/list/watch`).toBe(true)
    }
  }
  // Read-only role: no write verb anywhere.
  const verbs = new Set((role.rules ?? []).flatMap((r) => r.verbs ?? []))
  expect([...verbs].sort()).toEqual(['get', 'list', 'watch'])
})

for (const binding of BINDINGS) {
  test(`${label(binding)} -> ClusterRole/${binding.roleRef}`, async ({ page, app }) => {
    const existing = getRbac(binding.kind, binding.name, binding.namespace)
    if (existing) {
      // Idempotent: verify, never touch.
      expect(
        bindingMismatches(binding, existing),
        `${label(binding)} differs from the plan`,
      ).toEqual([])
      test.info().annotations.push({ type: 'result', description: 'already existed; verified' })
      return
    }

    const tap = await tapBridge(page)
    await app.open()
    await app.selectContext()
    if (binding.namespace) await app.selectNamespace(binding.namespace)
    await createThroughUi(page, tap, binding)

    const created = getRbac(binding.kind, binding.name, binding.namespace)
    expect(created, `${label(binding)} on the cluster`).not.toBeNull()
    if (created) expect(bindingMismatches(binding, created)).toEqual([])
    test.info().annotations.push({ type: 'result', description: 'created through the UI' })
  })
}

test('namespace-access lookup: agent is admin only in fathom-dev and fathom-e2e', async ({
  page,
  app,
}) => {
  const tap = await tapBridge(page)
  await app.open()
  await app.selectContext()
  await navigateTo(page, 'Namespace Access', 'Access Control')
  const view = page.getByTestId('rbac-namespace-access-lookup')
  await expect(view).toBeVisible()
  await view.getByTestId('rbac-namespace-access-kind').selectOption(AGENT.kind)
  await view.getByTestId('rbac-namespace-access-name').fill(AGENT.name)
  await view.getByTestId('rbac-namespace-access-subject-namespace').fill(AGENT.namespace)
  await view.getByTestId('rbac-namespace-access-submit').click()

  // Cluster-wide: read-only (view + fathom-agent-cluster-read).
  const clusterWide = view.getByTestId('rbac-namespace-access-clusterwide')
  await expect(clusterWide).toBeVisible()
  await expect(clusterWide).toContainText('fathom-agent-view')
  await expect(clusterWide).toContainText('fathom-agent-cluster-read')
  const clusterVerbs = (await clusterWide.locator('tbody tr td:first-child').allTextContents())
    .flatMap((t) => t.split(','))
    .map((v) => v.trim())
  expect(new Set(clusterVerbs), 'cluster-wide grants are read-only').toEqual(
    new Set(['get', 'list', 'watch']),
  )

  // Per-namespace: exactly the two fathom-* namespaces, via the admin RoleBindings.
  const perNs = view.getByTestId('rbac-namespace-access-per-namespace')
  await expect(perNs).toBeVisible()
  const namespaces = await perNs
    .locator('tbody tr td:first-child')
    .evaluateAll((cells) => [...new Set(cells.map((c) => c.textContent.trim()))].sort())
  expect(namespaces).toEqual([...AGENT_NAMESPACES].sort())
  await expect(perNs).toContainText('fathom-agent-admin')

  await page.screenshot({ path: test.info().outputPath('namespace-access.png'), fullPage: true })
  expect(tap.writes(), 'the lookup is read-only').toEqual([])
})

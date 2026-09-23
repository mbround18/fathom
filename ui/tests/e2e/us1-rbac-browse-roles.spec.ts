/**
 * US1 (011-rbac-oidc-administration, T010-T019): browsing Roles/ClusterRoles/RoleBindings/
 * ClusterRoleBindings against the fake backend — list rendering, opening an object shows its
 * detail sections (the Rules table for Roles/ClusterRoles, subjects for the bindings), and the
 * dangling-`roleRef` badge (`rbac-dangling-badge`) appears on a binding whose `roleRef` matches
 * no known Role/ClusterRole and not on a healthy one.
 *
 * Navigation: `access/roles`, `access/rolebindings`, `access/clusterroles`,
 * `access/clusterrolebindings` are already registered in `ui/src/resources/nav.ts` under
 * "Access Control" (T010's prerequisite), but the descriptors themselves
 * (`ui/src/resources/roles.tsx` etc.) are registered into the `descriptorFor`/`ALL_DESCRIPTORS`
 * registry (`ui/src/resources/index.ts`) by a separate integration pass that lands after this
 * spec is written (per this feature's parallel-workstream split — `index.ts` is intentionally not
 * touched here to avoid conflicting with that pass). Until that registration lands, the list pane
 * shows "Not ported yet" instead of the table; this spec is written to pass once it does, the
 * same convention `us4-rbac-namespace-access.spec.ts` uses for its own nav-wiring dependency.
 *
 * The degraded-scope-banner (`DegradedScopeBanner`, `rbac-degraded-scope`) is intentionally NOT
 * exercised here: wiring it into the generic list-error path (`PaneHost.tsx`'s `ListBody`, which
 * this feature's instructions say not to touch) is out of scope for US1 browsing. It is verified
 * as a component in isolation instead (`tests/unit/DegradedScopeBanner.test.tsx`).
 */
import { expect, type Page, test } from '@playwright/test'

import { batch, list, nsRoute, read } from '../fixtures/builders'
import { emit, expectRowCount, navigateTo, open, row, sent, sentTypes, table } from './helpers'

const ROLES = 'access/roles'
const ROLE_BINDINGS = 'access/rolebindings'
const CLUSTER_ROLES = 'access/clusterroles'
const CLUSTER_ROLE_BINDINGS = 'access/clusterrolebindings'

const POD_READER = { name: 'pod-reader', namespace: 'default', rules_count: 1 }
const VIEW = { name: 'view', rules_count: 3 }

const HEALTHY_BINDING = {
  name: 'pod-readers',
  namespace: 'default',
  role_ref: 'pod-reader',
  subjects_count: 1,
}
const DANGLING_BINDING = {
  name: 'ghost-binding',
  namespace: 'default',
  role_ref: 'no-such-role',
  subjects_count: 1,
}

async function goTo(page: Page, label: string, section: string): Promise<void> {
  await navigateTo(page, label, 'Access Control')
  await expect(table(page, section)).toBeVisible()
}

test.describe('US1 RBAC browse Roles/ClusterRoles/RoleBindings/ClusterRoleBindings', () => {
  test('T013: lists Roles with the expected columns and opening one shows the Rules table', async ({
    page,
  }) => {
    await open(page)
    await goTo(page, 'Roles', ROLES)
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoles')

    await emit(
      page,
      batch([list(1, nsRoute('kind-a', ROLES), { type: 'RolesUpdated', data: [POD_READER] })]),
    )
    await expectRowCount(page, 1, ROLES)
    const roleRow = row(page, `${POD_READER.namespace}/${POD_READER.name}`, ROLES)
    await expect(roleRow).toContainText('pod-reader')
    await expect(roleRow).toContainText('default')

    await roleRow.click()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['SelectRole']))
    expect((await sent(page)).find((c) => c.type === 'SelectRole')).toEqual({
      type: 'SelectRole',
      data: { namespace: POD_READER.namespace, name: POD_READER.name },
    })

    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', ROLES, POD_READER.namespace), {
          type: 'RoleDetailsUpdated',
          data: {
            name: POD_READER.name,
            namespace: POD_READER.namespace,
            creation_timestamp: '2024-01-01T00:00:00Z',
            labels: { team: 'platform' },
            annotations: {},
            rules: [{ api_groups: [], resources: ['pods'], verbs: ['get', 'list', 'watch'] }],
            raw_json: '{}',
          },
        }),
      ]),
    )

    const detail = page.locator('[data-testid="detail-sections"]').first()
    await expect(detail).toBeVisible()
    const ruleRow = detail.getByTestId('rbac-role-rule-row')
    await expect(ruleRow).toHaveCount(1)
    await expect(ruleRow).toContainText('(core)')
    await expect(ruleRow).toContainText('pods')
    await expect(ruleRow).toContainText('get, list, watch')
    await expect(detail).toContainText('team')
    await expect(detail).toContainText('platform')
  })

  test('T014: lists ClusterRoles', async ({ page }) => {
    await open(page)
    await goTo(page, 'Cluster Roles', CLUSTER_ROLES)
    await expect.poll(() => sentTypes(page)).toContain('RefreshClusterRoles')

    await emit(
      page,
      batch([
        list(
          1,
          { section: CLUSTER_ROLES, scope: 'Cluster', context: 'kind-a', namespace: null },
          {
            type: 'ClusterRolesUpdated',
            data: [VIEW],
          },
        ),
      ]),
    )
    await expectRowCount(page, 1, CLUSTER_ROLES)
    await expect(row(page, VIEW.name, CLUSTER_ROLES)).toContainText('view')
  })

  test('T015/T016: dangling roleRef badge appears on a RoleBinding whose roleRef resolves to nothing, and not on a healthy one', async ({
    page,
  }) => {
    await open(page)

    // Load the known Roles/ClusterRoles first (the dangling check resolves against them).
    await goTo(page, 'Roles', ROLES)
    await emit(
      page,
      batch([list(1, nsRoute('kind-a', ROLES), { type: 'RolesUpdated', data: [POD_READER] })]),
    )
    await expectRowCount(page, 1, ROLES)

    await goTo(page, 'Cluster Roles', CLUSTER_ROLES)
    await emit(
      page,
      batch([
        list(
          2,
          { section: CLUSTER_ROLES, scope: 'Cluster', context: 'kind-a', namespace: null },
          {
            type: 'ClusterRolesUpdated',
            data: [],
          },
        ),
      ]),
    )

    await goTo(page, 'Role Bindings', ROLE_BINDINGS)
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
    await emit(
      page,
      batch([
        list(3, nsRoute('kind-a', ROLE_BINDINGS), {
          type: 'RoleBindingsUpdated',
          data: [HEALTHY_BINDING, DANGLING_BINDING],
        }),
      ]),
    )
    await expectRowCount(page, 2, ROLE_BINDINGS)

    const healthyRow = row(
      page,
      `${HEALTHY_BINDING.namespace}/${HEALTHY_BINDING.name}`,
      ROLE_BINDINGS,
    )
    const danglingRow = row(
      page,
      `${DANGLING_BINDING.namespace}/${DANGLING_BINDING.name}`,
      ROLE_BINDINGS,
    )
    await expect(healthyRow.getByTestId('rbac-dangling-badge')).toHaveCount(0)
    await expect(danglingRow.getByTestId('rbac-dangling-badge')).toBeVisible()
    await expect(danglingRow.getByTestId('rbac-dangling-badge')).toContainText('Dangling')

    // The detail pane also surfaces the badge once the binding is opened.
    await danglingRow.click()
    await emit(
      page,
      batch([
        read(4, nsRoute('kind-a', ROLE_BINDINGS, DANGLING_BINDING.namespace), {
          type: 'RoleBindingDetailsUpdated',
          data: {
            name: DANGLING_BINDING.name,
            namespace: DANGLING_BINDING.namespace,
            creation_timestamp: '2024-01-01T00:00:00Z',
            labels: {},
            annotations: {},
            role_ref: DANGLING_BINDING.role_ref,
            subjects: [{ kind: 'User', name: 'jane', namespace: null }],
            raw_json: '{}',
            resource_version: '1',
          },
        }),
      ]),
    )
    const detail = page.locator('[data-testid="detail-sections"]').first()
    await expect(detail.getByTestId('rbac-dangling-badge')).toBeVisible()
    await expect(detail).toContainText('jane')
  })

  test('T017: lists ClusterRoleBindings', async ({ page }) => {
    await open(page)
    await goTo(page, 'Cluster Role Bindings', CLUSTER_ROLE_BINDINGS)
    await expect.poll(() => sentTypes(page)).toContain('RefreshClusterRoleBindings')

    const binding = { name: 'cluster-admins', role_ref: 'cluster-admin', subjects_count: 2 }
    await emit(
      page,
      batch([
        list(
          1,
          { section: CLUSTER_ROLE_BINDINGS, scope: 'Cluster', context: 'kind-a', namespace: null },
          { type: 'ClusterRoleBindingsUpdated', data: [binding] },
        ),
      ]),
    )
    await expectRowCount(page, 1, CLUSTER_ROLE_BINDINGS)
    await expect(row(page, binding.name, CLUSTER_ROLE_BINDINGS)).toContainText('cluster-admin')
    // No ClusterRoles list has been loaded in this test, so this roleRef cannot be resolved and
    // is shown as dangling.
    await expect(
      row(page, binding.name, CLUSTER_ROLE_BINDINGS).getByTestId('rbac-dangling-badge'),
    ).toBeVisible()
  })
})

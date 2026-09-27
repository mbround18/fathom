/**
 * 042-role-create: create and edit a Role/ClusterRole through `RoleForm`.
 *
 *  - create a Role: rules editor (discovery-fed pickers, verb checkboxes, presets, add/remove
 *    rows) -> review shows the manifest with Copy/Download -> confirm -> `CreateResourceYaml`,
 *    then "Bind this role…" opens the RoleBinding form prefilled with the new role.
 *  - Apply is disabled with the self-access review's reason when the write is denied, while the
 *    manifest can still be reviewed.
 *  - the API server's privilege-escalation error is explained, not just echoed.
 *  - edit an aggregated `system:` ClusterRole: both warnings, the diff, `ApplyResourceYaml` with
 *    the live resourceVersion.
 */
import { expect, type Locator, type Page, test } from '@playwright/test'

import { batch, clusterRoute, list, nsRoute, read, stream } from '../fixtures/builders'
import {
  clearSent,
  confirmDialog,
  emit,
  expectRowCount,
  finish,
  navigateTo,
  requestIdOf,
  open,
  row,
  sent,
  sentMutations,
  sentTypes,
  table,
} from './helpers'

const ROLES = 'access/roles'
const CLUSTER_ROLES = 'access/clusterroles'
const RBAC = 'rbac.authorization.k8s.io'

const CATALOG = {
  groups: [
    {
      group: '',
      resources: [
        { name: 'configmaps', namespaced: true, verbs: ['get', 'list'] },
        { name: 'nodes', namespaced: false, verbs: ['get', 'list'] },
        { name: 'pods', namespaced: true, verbs: ['get', 'list'] },
      ],
    },
    { group: 'apps', resources: [{ name: 'deployments', namespaced: true, verbs: ['get'] }] },
  ],
  failed_groups: ['metrics.k8s.io'],
}

const ESCALATION =
  'clusterroles.rbac.authorization.k8s.io "secret-reader" is forbidden: user "dev" (groups=["system:authenticated"]) is attempting to grant RBAC permissions not currently held:\n{APIGroups:[""], Resources:["secrets"], Verbs:["get"]}'

async function openRoles(page: Page, section: string, label: string): Promise<void> {
  await open(page)
  await navigateTo(page, label, 'Access Control')
  if (section === ROLES) {
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoles')
    await emit(page, batch([list(1, nsRoute('kind-a', ROLES), { type: 'RolesUpdated', data: [] })]))
  } else {
    await expect.poll(() => sentTypes(page)).toContain('RefreshClusterRoles')
    await emit(
      page,
      batch([
        list(1, clusterRoute('kind-a', CLUSTER_ROLES), {
          type: 'ClusterRolesUpdated',
          data: [{ name: 'system:aggregate-to-view', rules_count: 1 }],
        }),
      ]),
    )
  }
  await expect(table(page, section)).toBeVisible()
}

async function openCreate(page: Page, section: string, label: string): Promise<Locator> {
  await openRoles(page, section, label)
  await page.getByTestId('rbac-role-create-open').click()
  const form = page.getByTestId('rbac-role-form')
  await expect(form).toBeVisible()
  // The form asks for API discovery to feed its pickers.
  await expect.poll(() => sentTypes(page)).toContain('RefreshApiResources')
  await emit(
    page,
    batch([
      read(50, clusterRoute('kind-a', ROLES), { type: 'ApiResourcesUpdated', data: CATALOG }),
    ]),
  )
  return form
}

async function answerAccess(
  page: Page,
  verb: string,
  resource: string,
  namespace: string | null,
  allowed: boolean,
  reason: string | null = null,
): Promise<void> {
  await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
  await emit(
    page,
    batch([
      read(100, nsRoute('kind-a', ROLES), {
        type: 'RbacSelfAccessUpdated',
        data: { results: [{ check: { verb, resource, namespace, group: RBAC }, allowed, reason }] },
      }),
    ]),
  )
}

async function addToken(field: Locator, text: string): Promise<void> {
  await field.fill(text)
  await field.press('Enter')
}

test.describe('042 Role create/edit', () => {
  test('create a Role, review the manifest, apply, then bind it', async ({ page }) => {
    const form = await openCreate(page, ROLES, 'Roles')
    await form.getByTestId('rbac-role-name').fill('pod-reader')
    await form.getByTestId('rbac-role-namespace').fill('team-a')
    await answerAccess(page, 'create', 'roles', 'team-a', true)

    // One read-only row to start: get/list/watch ticked, the rest not.
    const rows = form.getByTestId('rbac-rule-row')
    await expect(rows).toHaveCount(1)
    await expect(form.getByTestId('rbac-role-preset')).toHaveValue('read-only')
    await expect(rows.first().getByTestId('rbac-rule-verb-list')).toBeChecked()
    await expect(rows.first().getByTestId('rbac-rule-verb-create')).not.toBeChecked()

    // Discovery feeds the resource picker; free text still works.
    await expect(rows.first().locator('datalist option[value="pods"]')).toHaveCount(1)
    await expect(rows.first().locator('datalist option[value="nodes"]')).toHaveCount(0)
    await addToken(rows.first().getByTestId('rbac-rule-resources-input'), 'pods, pods/log')
    await expect(rows.first().getByTestId('rbac-rule-resources-token')).toHaveCount(2)
    await expect(form.getByTestId('rbac-rule-discovery-partial')).toContainText('metrics.k8s.io')

    // Add a second rule, switch it to a custom verb set, then remove it again.
    await form.getByTestId('rbac-rule-add').click()
    await expect(rows).toHaveCount(2)
    await addToken(rows.nth(1).getByTestId('rbac-rule-api-groups-input'), 'apps')
    await addToken(rows.nth(1).getByTestId('rbac-rule-resources-input'), 'deployments')
    await rows.nth(1).getByTestId('rbac-rule-verb-patch').check()
    await expect(form.getByTestId('rbac-role-preset')).toHaveValue('custom')
    await rows.nth(1).getByTestId('rbac-rule-remove').click()
    await expect(rows).toHaveCount(1)
    await expect(form.getByTestId('rbac-role-preset')).toHaveValue('read-only')

    // Rows missing verbs block review.
    await rows.first().getByTestId('rbac-rule-verb-get').uncheck()
    await rows.first().getByTestId('rbac-rule-verb-list').uncheck()
    await rows.first().getByTestId('rbac-rule-verb-watch').uncheck()
    await expect(form.getByTestId('rbac-role-review')).toBeDisabled()
    await form.getByTestId('rbac-role-preset').selectOption('read-only')
    await expect(form.getByTestId('rbac-role-review')).toBeEnabled()

    await form.getByTestId('rbac-role-review').click()
    const manifest = form.getByTestId('rbac-role-manifest')
    await expect(manifest).toBeVisible()
    await expect(manifest).toContainText('kind: Role')
    await expect(manifest).toContainText('namespace: team-a')
    await expect(manifest).toContainText('- pods/log')
    await expect(form.getByTestId('manifest-copy')).toBeEnabled()
    await expect(form.getByTestId('manifest-download')).toBeEnabled()

    await clearSent(page)
    await form.getByTestId('yaml-apply').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('CreateResourceYaml')
    const id = await requestIdOf(page, 'CreateResourceYaml')
    const cmd = (await sent(page)).find((c) => c.type === 'CreateResourceYaml')
    expect(cmd).toMatchObject({
      data: {
        api_version: 'rbac.authorization.k8s.io/v1',
        kind: 'Role',
        namespace: 'team-a',
        name: 'pod-reader',
      },
    })
    expect(cmd?.type === 'CreateResourceYaml' && cmd.data.yaml).toContain('- watch')

    await emit(
      page,
      batch([
        stream(900, nsRoute('kind-a', ROLES, 'team-a'), {
          type: 'ResourceApplied',
          data: { request_id: String(id), resource_version: '1' },
        }),
      ]),
    )
    await finish(page, id, true)
    await expect(form.getByTestId('rbac-role-applied')).toBeVisible()
    await expectRowCount(page, 0, ROLES)

    // Bind shortcut: the RoleBinding form, prefilled with the new Role.
    await form.getByTestId('rbac-role-bind-open').click()
    const bindForm = page.getByTestId('rbac-binding-form')
    await expect(bindForm).toBeVisible()
    await expect(bindForm.getByTestId('rbac-binding-namespace')).toHaveValue('team-a')
    await expect(bindForm.getByTestId('rbac-binding-roleref-kind')).toHaveValue('Role')
    await expect(bindForm.getByTestId('rbac-binding-roleref-name')).toHaveValue('pod-reader')
    await expect(bindForm.getByTestId('rbac-binding-name')).toHaveValue('pod-reader-binding')
  })

  test('Apply is disabled with the reason when access is denied; the manifest is still reviewable', async ({
    page,
  }) => {
    const form = await openCreate(page, CLUSTER_ROLES, 'Cluster Roles')
    await answerAccess(
      page,
      'create',
      'clusterroles',
      null,
      false,
      'clusterroles is forbidden: User "dev" cannot create',
    )
    await expect(form.getByTestId('rbac-role-apply-disabled')).toContainText('forbidden')
    // No namespace field for a ClusterRole.
    await expect(form.getByTestId('rbac-role-namespace')).toHaveCount(0)

    await form.getByTestId('rbac-role-name').fill('fathom-e2e-viewer')
    await addToken(form.getByTestId('rbac-rule-resources-input'), 'nodes')
    await form.getByTestId('rbac-role-review').click()
    await expect(form.getByTestId('rbac-role-manifest')).toContainText('kind: ClusterRole')
    await expect(form.getByTestId('rbac-role-apply-disabled')).toBeVisible()
    await expect(form.getByTestId('yaml-apply')).toBeDisabled()

    await clearSent(page)
    await form.getByTestId('yaml-apply').click({ force: true })
    expect(await sentMutations(page)).toEqual([])
  })

  test('a privilege-escalation refusal is explained', async ({ page }) => {
    const form = await openCreate(page, CLUSTER_ROLES, 'Cluster Roles')
    await answerAccess(page, 'create', 'clusterroles', null, true)
    await form.getByTestId('rbac-role-name').fill('secret-reader')
    await addToken(form.getByTestId('rbac-rule-resources-input'), 'secrets')
    await form.getByTestId('rbac-role-review').click()

    await form.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('CreateResourceYaml')
    const id = await requestIdOf(page, 'CreateResourceYaml')
    await finish(page, id, false, ESCALATION)

    const explained = form.getByTestId('rbac-role-escalation')
    await expect(explained).toBeVisible()
    await expect(explained).toContainText('permissions you do not hold')
    await expect(explained).toContainText('Resources:["secrets"]')
    await expect(form.getByTestId('yaml-apply-error')).toContainText('attempting to grant')
    await expect(form.getByTestId('rbac-role-applied')).toHaveCount(0)
  })

  test('edit an aggregated system: ClusterRole: warnings, diff, ApplyResourceYaml', async ({
    page,
  }) => {
    await openRoles(page, CLUSTER_ROLES, 'Cluster Roles')
    const name = 'system:aggregate-to-view'
    await row(page, name, CLUSTER_ROLES).click()
    await emit(
      page,
      batch([
        read(2, clusterRoute('kind-a', CLUSTER_ROLES), {
          type: 'ClusterRoleDetailsUpdated',
          data: {
            name,
            creation_timestamp: '2024-01-01T00:00:00Z',
            labels: {},
            annotations: {},
            rules: [{ api_groups: [''], resources: ['pods'], verbs: ['get'] }],
            raw_json: JSON.stringify({
              apiVersion: 'rbac.authorization.k8s.io/v1',
              kind: 'ClusterRole',
              metadata: { name, resourceVersion: '42' },
              aggregationRule: { clusterRoleSelectors: [{ matchLabels: { x: 'y' } }] },
              rules: [
                { apiGroups: [''], resources: ['pods'], resourceNames: ['web'], verbs: ['get'] },
              ],
            }),
          },
        }),
      ]),
    )
    await page.getByTestId('rbac-role-edit-open').click()
    const form = page.getByTestId('rbac-role-form')
    await expect(form).toBeVisible()
    await answerAccess(page, 'update', 'clusterroles', null, true)

    await expect(form.getByTestId('rbac-role-warning-system')).toBeVisible()
    await expect(form.getByTestId('rbac-role-warning-aggregated')).toBeVisible()
    await expect(form.getByTestId('rbac-role-name')).toHaveValue(name)
    await expect(form.getByTestId('rbac-role-name')).toBeDisabled()
    // Pre-filled from raw_json, resourceNames included.
    await expect(form.getByTestId('rbac-rule-resource-names-token')).toHaveText('web')
    await expect(form.getByTestId('rbac-role-preset')).toHaveValue('custom')

    await form.getByTestId('rbac-rule-verb-list').check()
    await form.getByTestId('rbac-role-review').click()
    await expect(form.getByTestId('yaml-diff')).toBeVisible()
    await expect(form.getByTestId('yaml-diff')).toContainText('- list')

    await clearSent(page)
    await form.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('ApplyResourceYaml')
    const cmd = (await sent(page)).find((c) => c.type === 'ApplyResourceYaml')
    expect(cmd).toMatchObject({
      data: { kind: 'ClusterRole', namespace: null, name, expected_resource_version: '42' },
    })
    const yaml = cmd?.type === 'ApplyResourceYaml' ? cmd.data.yaml : ''
    expect(yaml).toContain('aggregationRule:')
    expect(yaml).toContain('- web')
  })
})

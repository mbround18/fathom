/**
 * US3 (011-rbac-oidc-administration, T030-T034): edit and delete an existing RoleBinding or
 * ClusterRoleBinding.
 *
 *  - (a) edit: open a binding, change its subjects via `RoleBindingForm` (edit mode), review ->
 *    `yaml-diff` shows only the subject delta (metadata/roleRef untouched) -> confirm ->
 *    `ApplyResourceYaml` is sent carrying `expected_resource_version` equal to the live object's
 *    `resource_version` (FR-005/T031).
 *  - (b) a stale-`resourceVersion` conflict on that apply is classified distinctly (`yaml-reload`
 *    shown, `yaml-apply-error` text), never silently overwriting the live object, and the edit
 *    buffer is preserved (mirrors `us1-apply-yaml.spec.ts` T014).
 *  - (c) delete: the generic delete-action/`ConfirmDialog` flow already wired for RoleBindings/
 *    ClusterRoleBindings (`ui/src/resources/rolebindings.tsx`/`clusterrolebindings.tsx`,
 *    `deleteAction`/hand-written `ActionDescriptor`) sends `DeleteRoleBinding`/
 *    `DeleteClusterRoleBinding` and the row is removed only once the backend's refreshed list
 *    arrives (mirrors `us3-bulk-delete.spec.ts`), never optimistically.
 *
 * `RoleBindingForm.tsx` (edit mode) and its `RoleBindingEditLauncher` (`ui/src/components/rbac/`)
 * are standalone per this workstream's instructions; the "Edit" button this spec clicks
 * (`rbac-binding-edit-open`) is dropped into the RoleBinding/ClusterRoleBinding detail view by a
 * separate integration pass, the same convention `us1-rbac-browse-roles.spec.ts` and
 * `us2-rbac-create-binding.spec.ts` document for their own entry points — this spec is written to
 * pass once that lands.
 */
import { expect, type Locator, type Page, test } from '@playwright/test'

import { batch, list, nsRoute, read, stream } from '../fixtures/builders'
import {
  check,
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
  pressDelete,
} from './helpers'

const ROLE_BINDINGS = 'access/rolebindings'
const CLUSTER_ROLE_BINDINGS = 'access/clusterrolebindings'

const BINDING_ROW = {
  name: 'view-secrets',
  namespace: 'team-a',
  role_ref: 'Role/secret-reader',
  subjects_count: 1,
}

const BINDING_DETAILS = {
  name: 'view-secrets',
  namespace: 'team-a',
  creation_timestamp: '2024-01-01T00:00:00Z',
  labels: { team: 'platform' },
  annotations: {},
  role_ref: 'Role/secret-reader',
  subjects: [{ kind: 'User', name: 'alice', namespace: null }],
  raw_json: JSON.stringify({
    apiVersion: 'rbac.authorization.k8s.io/v1',
    kind: 'RoleBinding',
    metadata: {
      name: 'view-secrets',
      namespace: 'team-a',
      resourceVersion: '42',
      labels: { team: 'platform' },
    },
    roleRef: { apiGroup: 'rbac.authorization.k8s.io', kind: 'Role', name: 'secret-reader' },
    subjects: [{ kind: 'User', name: 'alice' }],
  }),
  resource_version: '42',
}

async function openBindingDetail(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Role Bindings', 'Access Control')
  // The pane shows a loading state (not the `resource-table` testid) until the list event below
  // lands, so this waits for the Refresh command and emits before asserting row count.
  await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
  await emit(
    page,
    batch([
      list(1, nsRoute('kind-a', ROLE_BINDINGS), {
        type: 'RoleBindingsUpdated',
        data: [BINDING_ROW],
      }),
    ]),
  )
  await expectRowCount(page, 1, ROLE_BINDINGS)
  await row(page, `${BINDING_ROW.namespace}/${BINDING_ROW.name}`, ROLE_BINDINGS).click()
  await emit(
    page,
    batch([
      read(2, nsRoute('kind-a', ROLE_BINDINGS, BINDING_ROW.namespace), {
        type: 'RoleBindingDetailsUpdated',
        data: BINDING_DETAILS,
      }),
    ]),
  )
}

async function openEditForm(page: Page): Promise<Locator> {
  await openBindingDetail(page)
  await page.getByTestId('rbac-binding-edit-open').click()
  const form = page.getByTestId('rbac-binding-form')
  await expect(form).toBeVisible()
  await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
  await emit(
    page,
    batch([
      read(3, nsRoute('kind-a', ROLE_BINDINGS, BINDING_ROW.namespace), {
        type: 'RbacSelfAccessUpdated',
        data: {
          results: [
            {
              check: {
                verb: 'update',
                resource: 'rolebindings',
                namespace: BINDING_ROW.namespace,
                group: 'rbac.authorization.k8s.io',
              },
              allowed: true,
              reason: null,
            },
          ],
        },
      }),
    ]),
  )
  return form
}

test.describe('US3 edit/delete RoleBinding', () => {
  test('(a) T031/T034: edit a subject, review shows only the delta, apply carries expected_resource_version', async ({
    page,
  }) => {
    const form = await openEditForm(page)

    // Existing subject's name field, plus the Add-subject control.
    await form.getByTestId('rbac-subject-name').first().fill('bob')

    const review = form.getByTestId('rbac-binding-review')
    await expect(review).toBeEnabled()
    await review.click()

    const diff = form.getByTestId('yaml-diff')
    await expect(diff).toBeVisible()
    await expect(diff).toContainText('bob')
    // Untouched fields are preserved by the patch (not regenerated from scratch).
    await expect(diff).toContainText('resourceVersion: "42"')
    await expect(diff).toContainText('team: platform')

    await clearSent(page)
    await form.getByTestId('yaml-apply').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['ApplyResourceYaml']))
    const cmd = (await sent(page)).find((c) => c.type === 'ApplyResourceYaml')
    expect(cmd).toMatchObject({
      type: 'ApplyResourceYaml',
      data: {
        kind: 'RoleBinding',
        namespace: BINDING_ROW.namespace,
        name: BINDING_ROW.name,
        expected_resource_version: BINDING_DETAILS.resource_version,
      },
    })

    const id = await requestIdOf(page, 'ApplyResourceYaml')

    // The session only reaches `applied` once the `ResourceApplied` stream event lands (the same
    // event `CommandFinished` accompanies for every apply — mirrors `us1-apply-yaml.spec.ts`).
    await emit(
      page,
      batch([
        stream(900, nsRoute('kind-a', ROLE_BINDINGS, BINDING_ROW.namespace), {
          type: 'ResourceApplied',
          data: { request_id: String(id), resource_version: '43' },
        }),
      ]),
    )
    await finish(page, id, true)
    await expect(form.getByTestId('rbac-binding-applied')).toBeVisible()
  })

  test('(b) a stale-resourceVersion conflict is surfaced, never silently overwritten, buffer preserved', async ({
    page,
  }) => {
    const form = await openEditForm(page)
    await form.getByTestId('rbac-subject-name').first().fill('bob')
    await form.getByTestId('rbac-binding-review').click()
    await expect(form.getByTestId('yaml-diff')).toBeVisible()

    await form.getByTestId('yaml-apply').click()
    await expect(confirmDialog(page)).toBeVisible()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    const id = await requestIdOf(page, 'ApplyResourceYaml')

    await finish(
      page,
      id,
      false,
      'conflict: Operation cannot be fulfilled: the object has been modified',
    )

    await expect(form.getByTestId('yaml-apply-error')).toContainText(
      'Operation cannot be fulfilled',
    )
    await expect(form.getByTestId('yaml-reload')).toBeVisible()
    // Not applied, not silently discarded.
    await expect(form.getByTestId('rbac-binding-applied')).toHaveCount(0)
    await expect(form.getByTestId('yaml-diff')).toContainText('bob')
  })

  test('(c) delete: ConfirmDialog names the binding, DeleteRoleBinding sent, row removed only after refresh', async ({
    page,
  }) => {
    await open(page)
    await navigateTo(page, 'Role Bindings', 'Access Control')
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', ROLE_BINDINGS), {
          type: 'RoleBindingsUpdated',
          data: [BINDING_ROW],
        }),
      ]),
    )
    await expectRowCount(page, 1, ROLE_BINDINGS)

    const key = `${BINDING_ROW.namespace}/${BINDING_ROW.name}`
    await check(page, key, ROLE_BINDINGS)
    await clearSent(page)
    await pressDelete(page)

    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-item')).toHaveText([`✖${key}`])

    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sentMutations(page))
      .toEqual([
        {
          type: 'DeleteRoleBinding',
          data: { namespace: BINDING_ROW.namespace, name: BINDING_ROW.name },
        },
      ])

    // No optimistic removal.
    await expectRowCount(page, 1, ROLE_BINDINGS)
    await finish(page, await requestIdOf(page, 'DeleteRoleBinding'), true)
    await emit(
      page,
      batch([list(2, nsRoute('kind-a', ROLE_BINDINGS), { type: 'RoleBindingsUpdated', data: [] })]),
    )
    await expectRowCount(page, 0, ROLE_BINDINGS)
  })

  test('(c) ClusterRoleBinding delete: cluster-scoped DeleteClusterRoleBinding, no namespace', async ({
    page,
  }) => {
    const binding = {
      name: 'cluster-admins',
      role_ref: 'ClusterRole/cluster-admin',
      subjects_count: 1,
    }
    await open(page)
    await navigateTo(page, 'Cluster Role Bindings', 'Access Control')
    await expect.poll(() => sentTypes(page)).toContain('RefreshClusterRoleBindings')
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

    await check(page, binding.name, CLUSTER_ROLE_BINDINGS)
    await clearSent(page)
    await pressDelete(page)
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(() => sentMutations(page))
      .toEqual([{ type: 'DeleteClusterRoleBinding', data: { name: binding.name } }])

    await expectRowCount(page, 1, CLUSTER_ROLE_BINDINGS)
    await emit(
      page,
      batch([
        list(
          2,
          { section: CLUSTER_ROLE_BINDINGS, scope: 'Cluster', context: 'kind-a', namespace: null },
          { type: 'ClusterRoleBindingsUpdated', data: [] },
        ),
      ]),
    )
    await expectRowCount(page, 0, CLUSTER_ROLE_BINDINGS)
  })
})

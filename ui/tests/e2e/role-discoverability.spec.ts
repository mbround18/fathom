/**
 * 043 US8 (T080): 042's role form is reachable from where role access is managed.
 *
 *  - The RoleBindings toolbar has "New Role…" and "New ClusterRole…" (the ClusterRoleBindings one
 *    only "New ClusterRole…"), gated like 042's launcher.
 *  - The RoleBinding form's and the Assign role dialog's "Create a new role…" open the role form
 *    and come back with the new role as the binding's roleRef.
 *  - Namespace Access offers "Create a role…" next to its answer.
 *  - The command palette finds "New Role…" / "New ClusterRole…" for "role".
 */
import { expect, type Locator, type Page, test } from '@playwright/test'

import type { NamespaceAccessResolved } from '../../src/protocol/types'
import { batch, clusterRoute, list, nsRoute, read, stream, system } from '../fixtures/builders'
import {
  confirmDialog,
  emit,
  finish,
  navigateTo,
  open,
  requestIdOf,
  sent,
  sentTypes,
  table,
} from './helpers'

const ROLE_BINDINGS = 'access/rolebindings'
const CLUSTER_ROLE_BINDINGS = 'access/clusterrolebindings'
const ROLES = 'access/roles'
const RBAC = 'rbac.authorization.k8s.io'

// Radix unmounts a closing dialog only after its exit animation; reduced motion keeps the
// palette's close synchronous (same as us1-command-palette.spec.ts).
test.use({ reducedMotion: 'reduce' })

function denyCapabilities(page: Page, deny: { verb: string; resource: string; reason: string }[]) {
  return page.evaluate((d) => {
    ;(
      window as unknown as { __fathomFake: { setCapabilities(c: unknown): void } }
    ).__fathomFake.setCapabilities({ deny: d })
  }, deny)
}

async function openRoleBindings(page: Page): Promise<void> {
  await navigateTo(page, 'Role Bindings', 'Access Control')
  await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
  await emit(
    page,
    batch([list(1, nsRoute('kind-a', ROLE_BINDINGS), { type: 'RoleBindingsUpdated', data: [] })]),
  )
  await expect(table(page, ROLE_BINDINGS)).toBeVisible()
}

/** Answer a form's self-access review (`useMutationAccess`) for one check. */
async function answerAccess(
  page: Page,
  seq: number,
  resource: string,
  namespace: string | null,
): Promise<void> {
  await emit(
    page,
    batch([
      read(seq, nsRoute('kind-a', ROLES), {
        type: 'RbacSelfAccessUpdated',
        data: {
          results: [{ check: { verb: 'create', resource, namespace, group: RBAC }, allowed: true }],
        },
      } as never),
    ]),
  )
}

/** Fill 042's role form with one read-only rule, review, apply and confirm; the create lands. */
async function createRole(
  page: Page,
  form: Locator,
  kind: 'Role' | 'ClusterRole',
  namespace: string | null,
): Promise<void> {
  await form.getByTestId('rbac-role-name').fill('pod-reader')
  if (namespace) await form.getByTestId('rbac-role-namespace').fill(namespace)
  await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
  await answerAccess(page, 300, kind === 'Role' ? 'roles' : 'clusterroles', namespace)
  const resources = form
    .getByTestId('rbac-rule-row')
    .first()
    .getByTestId('rbac-rule-resources-input')
  await resources.fill('pods')
  await resources.press('Enter')
  await form.getByTestId('rbac-role-review').click()
  await expect(form.getByTestId('rbac-role-manifest')).toContainText(`kind: ${kind}`)
  await form.getByTestId('yaml-apply').click()
  await confirmDialog(page).getByTestId('confirm-submit').click()
  await expect
    .poll(async () => (await sent(page)).find((c) => c.type === 'CreateResourceYaml'))
    .toMatchObject({ data: { kind, name: 'pod-reader' } })
  const id = await requestIdOf(page, 'CreateResourceYaml')
  await emit(
    page,
    batch([
      stream(900, nsRoute('kind-a', ROLES, namespace), {
        type: 'ResourceApplied',
        data: { request_id: String(id), resource_version: '1' },
      }),
    ]),
  )
  await finish(page, id, true)
}

const ISSUER = {
  id: 'rauthy',
  issuer_url: 'https://auth.example.com/auth/v1',
  client_id: 'fathom',
  scopes: ['openid'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

test.describe('043 US8 role wizard discoverability', () => {
  test('RoleBindings toolbar: New Role… and New ClusterRole… open the role form', async ({
    page,
  }) => {
    await open(page)
    await openRoleBindings(page)
    const newRole = page.getByTestId('rbac-bindings-new-role')
    const newClusterRole = page.getByTestId('rbac-bindings-new-clusterrole')
    await expect(newRole).toHaveText('New Role…')
    await expect(newClusterRole).toHaveText('New ClusterRole…')
    await expect(newClusterRole).toBeEnabled()
    await expect(newRole).toBeEnabled()
    await newRole.click()
    await expect(page.getByTestId('rbac-role-form')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'New Role' })).toBeVisible()
  })

  test('ClusterRoleBindings toolbar offers New ClusterRole…', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Cluster Role Bindings', 'Access Control')
    await expect.poll(() => sentTypes(page)).toContain('RefreshClusterRoleBindings')
    await emit(
      page,
      batch([
        list(1, clusterRoute('kind-a', CLUSTER_ROLE_BINDINGS), {
          type: 'ClusterRoleBindingsUpdated',
          data: [],
        }),
      ]),
    )
    await expect(table(page, CLUSTER_ROLE_BINDINGS)).toBeVisible()
    const newClusterRole = page.getByTestId('rbac-bindings-new-clusterrole')
    await expect(newClusterRole).toBeEnabled()
    await newClusterRole.click()
    await expect(page.getByRole('heading', { name: 'New ClusterRole' })).toBeVisible()
  })

  test('the toolbar entries keep 042 gating: a denied create is disabled with its reason', async ({
    page,
  }) => {
    await open(page)
    await denyCapabilities(page, [
      { verb: 'create', resource: 'roles', reason: 'cannot create roles' },
    ])
    await openRoleBindings(page)
    const newRole = page.getByTestId('rbac-bindings-new-role')
    await expect(newRole).toBeDisabled()
    await expect(newRole).toHaveAttribute('title', 'cannot create roles')
    await expect(page.getByTestId('rbac-bindings-new-clusterrole')).toBeEnabled()
  })

  test('RoleBinding form: Create a new role… → pod-reader becomes the roleRef', async ({
    page,
  }) => {
    await open(page)
    await openRoleBindings(page)
    await page.getByTestId('rbac-binding-create-open').click()
    const binding = page.getByTestId('rbac-binding-form')
    await expect(binding).toBeVisible()
    await binding.getByTestId('rbac-binding-namespace').fill('team-a')
    await binding.getByTestId('rbac-binding-name').fill('pod-readers')
    await binding.getByTestId('rbac-binding-roleref-kind').selectOption('Role')

    const option = binding.getByTestId('rbac-binding-create-role')
    await expect(option).toHaveText('Create a new role…')
    await option.click()
    const form = page.getByTestId('rbac-role-form')
    await expect(form).toBeVisible()
    await expect(form.getByTestId('rbac-role-namespace')).toHaveValue('team-a')
    await createRole(page, form, 'Role', 'team-a')

    // Back on the binding, with what was filled in kept and the new role as roleRef.
    await expect(form).toBeHidden()
    await expect(binding.getByTestId('rbac-binding-roleref-kind')).toHaveValue('Role')
    await expect(binding.getByTestId('rbac-binding-roleref-name')).toHaveValue('pod-reader')
    await expect(binding.getByTestId('rbac-binding-name')).toHaveValue('pod-readers')
  })

  test('Assign role: Create a new role… → pod-reader becomes the roleRef', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Role Bindings', 'Access Control')
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', ROLE_BINDINGS), { type: 'RoleBindingsUpdated', data: [] }),
        system(2, {
          type: 'OidcConfigLoaded',
          data: { config: { issuers: [ISSUER], contexts: [], role_map: {} }, identities: [] },
        } as never),
      ]),
    )
    await page.getByTestId('role-assignment-open').click()
    const dialog = page.getByTestId('role-assignment-dialog')
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('role-ref-kind')).toHaveValue('ClusterRole')
    await dialog.getByTestId('role-assignment-create-role').click()
    const form = page.getByTestId('rbac-role-form')
    await expect(form).toBeVisible()
    await createRole(page, form, 'ClusterRole', null)

    await expect(form).toBeHidden()
    await expect(dialog.getByTestId('role-ref-kind')).toHaveValue('ClusterRole')
    await expect(dialog.getByTestId('role-ref-name')).toHaveValue('pod-reader')
  })

  test('Namespace Access offers Create a role… next to its answer', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Namespace Access', 'Access Control')
    const view = page.getByTestId('rbac-namespace-access-lookup')
    await view.getByTestId('rbac-namespace-access-kind').selectOption('User')
    await view.getByTestId('rbac-namespace-access-name').fill('jane')
    await view.getByTestId('rbac-namespace-access-submit').click()
    const resolved: NamespaceAccessResolved = {
      subject_kind: 'User',
      subject_name: 'jane',
      subject_namespace: null,
      cluster_wide: [],
      per_namespace: [],
      dangling_bindings: [],
      degraded_scopes: [],
      found_any: false,
    }
    await emit(
      page,
      batch([
        read(400, clusterRoute('kind-a', 'access/namespace-access'), {
          type: 'NamespaceAccessResolved',
          data: resolved,
        }),
      ]),
    )
    const hint = view.getByTestId('rbac-namespace-access-create-role')
    await expect(hint).toContainText('Then bind it to this user.')
    await hint.getByTestId('rbac-namespace-access-create-role-open').click()
    await expect(page.getByTestId('rbac-role-form')).toBeVisible()
  })

  for (const { label, heading, section, event } of [
    { label: 'New Role…', heading: 'New Role', section: ROLES, event: 'RolesUpdated' },
    {
      label: 'New ClusterRole…',
      heading: 'New ClusterRole',
      section: 'access/clusterroles',
      event: 'ClusterRolesUpdated',
    },
  ] as const) {
    test(`the palette finds ${label} for "role" and opens the form`, async ({ page }) => {
      await open(page)
      await page.keyboard.press('Control+k')
      await page.locator('[cmdk-input]').fill('role')
      const item = page.locator('[cmdk-item]', { hasText: label })
      await expect(item).toBeVisible()
      await item.click()
      await expect
        .poll(() => sentTypes(page))
        .toContain(section === ROLES ? 'RefreshRoles' : 'RefreshClusterRoles')
      const route = section === ROLES ? nsRoute('kind-a', section) : clusterRoute('kind-a', section)
      await emit(page, batch([list(1, route, { type: event, data: [] })]))
      await expect(page.getByTestId('rbac-role-form')).toBeVisible()
      await expect(page.getByRole('heading', { name: heading })).toBeVisible()
    })
  }
})

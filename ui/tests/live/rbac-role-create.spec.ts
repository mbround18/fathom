/**
 * LIVE (042-role-create): create, edit and delete a Role in `fathom-dev` and a `fathom-e2e-*`
 * ClusterRole through the UI, checking each step against the API server with kubectl. The
 * devbridge only lets a cluster-scoped write through for a `fathom-e2e-*` ClusterRole, and both
 * objects are deleted again in `afterEach` whatever the outcome.
 */
import type { Locator, Page } from '@playwright/test'

import {
  e2eClusterRole,
  expect,
  isPhone,
  kubectl,
  LIVE_NAMESPACE,
  navigateTo,
  row,
  search,
  table,
  test,
  type KubeObject,
} from './helpers'

const ROLES = 'access/roles'
const CLUSTER_ROLES = 'access/clusterroles'

interface Rule {
  apiGroups?: string[]
  resources?: string[]
  verbs: string[]
}

function rulesOf(obj: KubeObject | undefined): Rule[] {
  return (obj as { rules?: Rule[] } | undefined)?.rules ?? []
}

function getRole(name: string): KubeObject | undefined {
  const out = kubectl(['get', 'role', name, '--ignore-not-found', '-o', 'json'])
  return out.trim() ? (JSON.parse(out) as KubeObject) : undefined
}

async function addToken(field: Locator, text: string): Promise<void> {
  await field.fill(text)
  await field.press('Enter')
}

/** Review, confirm and wait until the API server accepted the write. */
async function reviewAndApply(page: Page, form: Locator): Promise<void> {
  await form.getByTestId('rbac-role-review').click()
  await expect(form.getByTestId('yaml-apply')).toBeEnabled({ timeout: 30_000 })
  await form.getByTestId('yaml-apply').click()
  await page.getByTestId('confirm-dialog').getByTestId('confirm-submit').click()
  await expect(form.getByTestId('rbac-role-applied')).toBeVisible({ timeout: 30_000 })
}

async function closeSheet(page: Page): Promise<void> {
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('rbac-role-form')).toHaveCount(0)
}

async function editVerb(page: Page, r: Locator, verb: string): Promise<void> {
  await r.click()
  await page.getByTestId('rbac-role-edit-open').click()
  const form = page.getByTestId('rbac-role-form')
  await expect(form).toBeVisible()
  await form.getByTestId(`rbac-rule-verb-${verb}`).check()
  await reviewAndApply(page, form)
  await closeSheet(page)
}

async function deleteRow(page: Page, r: Locator, rowKey: string): Promise<void> {
  const box = r.getByRole('checkbox', { name: `Select ${rowKey}` })
  await box.click()
  await expect(box).toHaveAttribute('aria-checked', 'true')
  await page.getByTestId('bulk-delete').click()
  const dialog = page.getByTestId('confirm-dialog')
  await expect(dialog.getByTestId('confirm-item')).toHaveCount(1)
  await dialog.getByTestId('confirm-submit').click()
}

test.describe('042 Role/ClusterRole create, edit, delete (live)', () => {
  test.beforeEach(() => {
    test.skip(isPhone(test.info()), 'wide layouts only: the sheet flow is the same')
  })

  const stamp = Date.now().toString(36)
  const roleName = `fathom-e2e-role-${stamp}`
  const clusterRoleName = `fathom-e2e-cr-${stamp}`

  test.afterEach(() => {
    kubectl(['delete', 'role', roleName, '--ignore-not-found'])
    e2eClusterRole('delete', clusterRoleName)
  })

  test('a Role in fathom-dev', async ({ app, page }) => {
    await app.open()
    await app.selectNamespace(LIVE_NAMESPACE)
    await navigateTo(page, 'Roles', 'Access Control')
    await expect(table(page, ROLES)).toBeVisible({ timeout: 30_000 })

    // Create.
    await page.getByTestId('rbac-role-create-open').click()
    const form = page.getByTestId('rbac-role-form')
    await expect(form).toBeVisible()
    // Real discovery feeds the resource picker (core, namespaced only for a Role).
    const rule = form.getByTestId('rbac-rule-row').first()
    await expect(rule.locator('datalist option[value="configmaps"]')).toHaveCount(1, {
      timeout: 30_000,
    })
    await expect(rule.locator('datalist option[value="nodes"]')).toHaveCount(0)
    await form.getByTestId('rbac-role-name').fill(roleName)
    await form.getByTestId('rbac-role-namespace').fill(LIVE_NAMESPACE)
    await addToken(rule.getByTestId('rbac-rule-resources-input'), 'configmaps')
    await reviewAndApply(page, form)
    expect(rulesOf(getRole(roleName))).toEqual([
      { apiGroups: [''], resources: ['configmaps'], verbs: ['get', 'list', 'watch'] },
    ])
    await closeSheet(page)

    // Edit: add `create`.
    await search(page, roleName, ROLES)
    const r = row(page, `${LIVE_NAMESPACE}/${roleName}`, ROLES)
    await expect(r).toBeVisible({ timeout: 30_000 })
    await editVerb(page, r, 'create')
    expect(rulesOf(getRole(roleName))[0]?.verbs).toEqual(['get', 'list', 'watch', 'create'])

    // Delete.
    await deleteRow(page, r, `${LIVE_NAMESPACE}/${roleName}`)
    await expect.poll(() => getRole(roleName), { timeout: 30_000 }).toBeUndefined()
    await expect(r).toHaveCount(0, { timeout: 30_000 })
  })

  test('a fathom-e2e-* ClusterRole', async ({ app, page }) => {
    await app.open()
    await navigateTo(page, 'Cluster Roles', 'Access Control')
    await expect(table(page, CLUSTER_ROLES)).toBeVisible({ timeout: 30_000 })

    await page.getByTestId('rbac-role-create-open').click()
    const form = page.getByTestId('rbac-role-form')
    await expect(form).toBeVisible()
    await expect(form.getByTestId('rbac-role-namespace')).toHaveCount(0)
    const rule = form.getByTestId('rbac-rule-row').first()
    // A ClusterRole also offers cluster-scoped resources.
    await expect(rule.locator('datalist option[value="nodes"]')).toHaveCount(1, {
      timeout: 30_000,
    })
    await form.getByTestId('rbac-role-name').fill(clusterRoleName)
    await addToken(rule.getByTestId('rbac-rule-resources-input'), 'nodes')
    await reviewAndApply(page, form)
    expect(rulesOf(e2eClusterRole('get', clusterRoleName))).toEqual([
      { apiGroups: [''], resources: ['nodes'], verbs: ['get', 'list', 'watch'] },
    ])
    // The bind shortcut is offered after a create (not followed here: no binding is made).
    await expect(form.getByTestId('rbac-role-bind-open')).toBeVisible()
    await closeSheet(page)

    await search(page, clusterRoleName, CLUSTER_ROLES)
    const r = row(page, clusterRoleName, CLUSTER_ROLES)
    await expect(r).toBeVisible({ timeout: 30_000 })
    await editVerb(page, r, 'patch')
    expect(rulesOf(e2eClusterRole('get', clusterRoleName))[0]?.verbs).toEqual([
      'get',
      'list',
      'watch',
      'patch',
    ])

    await deleteRow(page, r, clusterRoleName)
    await expect
      .poll(() => e2eClusterRole('get', clusterRoleName), { timeout: 30_000 })
      .toBeUndefined()
  })
})

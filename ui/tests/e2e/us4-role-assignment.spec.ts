/**
 * 033 US4: assign a role to an IdP group (preview YAML -> diff -> confirm -> create), a
 * conflict on apply, the denied-by-RBAC disabled state, and the read-only orphan badge.
 */
import { expect, type Page, test } from '@playwright/test'

import { batch, list, nsRoute, system } from '../fixtures/builders'
import {
  clearSent,
  confirmDialog,
  emit,
  finish,
  navigateTo,
  nextRequestId,
  open,
  sent,
  sentTypes,
  table,
} from './helpers'

const RB = 'access/rolebindings'

const issuer = {
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

const YAML = `apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: editors-edit
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: edit
subjects:
- apiGroup: rbac.authorization.k8s.io
  kind: Group
  name: oidc:fathom-editors
`

async function setup(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Role Bindings', 'Access Control')
  await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
  await emit(
    page,
    batch([
      list(1, nsRoute('kind-a', RB), {
        type: 'RoleBindingsUpdated',
        data: [
          { name: 'old', namespace: 'team-a', role_ref: 'ClusterRole/view', subjects_count: 1 },
        ],
      }),
      system(2, {
        type: 'OidcConfigLoaded',
        data: { config: { issuers: [issuer], contexts: [], role_map: {} }, identities: [] },
      } as never),
      system(3, {
        type: 'IdpDirectoryUpdated',
        data: {
          issuer_id: 'rauthy',
          users: [],
          groups: [
            { name: 'fathom-editors', member_count: 2 },
            { name: 'fathom-viewers', member_count: 3 },
          ],
          fetched_at: 10,
          stale: false,
          error: null,
        },
      } as never),
    ]),
  )
  await expect(table(page, RB)).toBeVisible()
}

async function fillAndPreview(page: Page): Promise<void> {
  await page.getByTestId('role-assignment-open').click()
  await expect(page.getByTestId('role-assignment-dialog')).toBeVisible()
  await page.getByTestId('role-subject-option-fathom-editors').click()
  await page.getByTestId('role-ref-name').fill('edit')
  await page.getByTestId('role-binding-name').fill('editors-edit')
  // The dialog is namespace-scoped when a namespace is active; either way the backend renders.
  await clearSent(page)
  await page.getByTestId('role-preview').click()
  await expect
    .poll(async () => (await sent(page)).find((c) => c.type === 'PreviewRoleAssignment'))
    .toMatchObject({
      data: {
        draft: {
          issuer_id: 'rauthy',
          subject: { type: 'Group', data: { name: 'fathom-editors' } },
        },
      },
    })
  await emit(
    page,
    batch([
      system(20, {
        type: 'RoleAssignmentPreview',
        data: {
          yaml: YAML,
          rendered_subject_name: 'oidc:fathom-editors',
          warnings: [],
        },
      } as never),
    ]),
  )
  await expect(page.getByTestId('yaml-diff')).toContainText('oidc:fathom-editors')
  await expect(page.getByTestId('role-rendered-subject')).toHaveText('oidc:fathom-editors')
}

test.describe('033 US4 role assignment', () => {
  test('assign: preview, confirm, create is sent', async ({ page }) => {
    await setup(page)
    await fillAndPreview(page)
    await clearSent(page)
    const id = await nextRequestId(page)
    await page.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'CreateResourceYaml'))
      .toMatchObject({ data: { kind: 'ClusterRoleBinding', name: 'editors-edit' } })
    await finish(page, id, true)
  })

  test('conflict on apply shows the error and keeps Apply disabled', async ({ page }) => {
    await setup(page)
    await fillAndPreview(page)
    const id = await nextRequestId(page)
    await page.getByTestId('yaml-apply').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await finish(page, id, false, 'conflict: already exists')
    await expect(page.getByTestId('yaml-apply-error')).toContainText('conflict')
    await expect(page.getByTestId('yaml-apply')).toBeDisabled()
  })

  test('denied by RBAC: the assign control is disabled with the reason', async ({ page }) => {
    await open(page)
    await page.evaluate(() => {
      ;(
        window as unknown as {
          __fathomFake: { setCapabilities(c: unknown): void }
        }
      ).__fathomFake.setCapabilities({
        deny: [{ verb: 'create', resource: 'rolebindings', reason: 'cannot create rolebindings' }],
      })
    })
    await navigateTo(page, 'Role Bindings', 'Access Control')
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', RB), { type: 'RoleBindingsUpdated', data: [] }),
        system(2, {
          type: 'OidcConfigLoaded',
          data: { config: { issuers: [issuer], contexts: [], role_map: {} }, identities: [] },
        } as never),
      ]),
    )
    const btn = page.getByTestId('role-assignment-open')
    await expect(btn).toBeDisabled()
    await expect(btn).toHaveAttribute('title', 'cannot create rolebindings')
  })

  test('orphaned subject badge is shown, read-only', async ({ page }) => {
    await setup(page)
    await emit(
      page,
      batch([
        system(30, {
          type: 'OrphanedSubjectsUpdated',
          data: { orphans: [{ binding: 'RoleBinding/team-a/old', subject_name: 'oidc:gone' }] },
        } as never),
      ]),
    )
    await expect(page.getByTestId('orphaned-subject-badge')).toContainText('oidc:gone')
    await clearSent(page)
    expect((await sent(page)).filter((c) => /Delete|Apply|Create/.test(c.type))).toEqual([])
  })
})

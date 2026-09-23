/**
 * US2 (011-rbac-oidc-administration, T023): create a RoleBinding via `RoleBindingForm` — fill
 * form -> `rbac-binding-review` opens `yaml-diff` (reused testid) -> `yaml-apply` opens
 * `ConfirmDialog` -> `confirm-submit` -> `CreateResourceYaml` is sent with the correct fields and
 * the binding list updates only after a subsequent `RoleBindingsUpdated` event (never
 * optimistically). Separate tests cover cancel (zero commands sent) and the create entry point
 * disabled behind a scripted `RbacSelfAccessUpdated { allowed: false }`.
 *
 * Mirrors `us1-apply-yaml.spec.ts`'s structure/helpers. `RoleBindingForm.tsx` and its
 * `RoleBindingCreateLauncher` (`ui/src/components/rbac/`) are standalone, self-contained
 * components per this workstream's instructions (never importing from, or imported by,
 * `ui/src/resources/rolebindings.tsx`/`index.ts` here) — the "New RoleBinding" button this spec
 * clicks (`rbac-binding-create-open`) is dropped into the RoleBindings list pane by a separate
 * integration pass (the same convention `us1-rbac-browse-roles.spec.ts` documents for
 * `index.ts`'s own registration), so this spec is written to pass once that lands.
 */
import { expect, type Page, test } from '@playwright/test'

import { batch, list, nsRoute, read } from '../fixtures/builders'
import {
  clearSent,
  confirmDialog,
  emit,
  expectRowCount,
  finish,
  navigateTo,
  nextRequestId,
  open,
  row,
  sent,
  sentTypes,
  table,
} from './helpers'

const ROLE_BINDINGS = 'access/rolebindings'

async function openCreateForm(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Role Bindings', 'Access Control')
  await expect(table(page, ROLE_BINDINGS)).toBeVisible()
  await page.getByTestId('rbac-binding-create-open').click()
  await expect(page.getByTestId('rbac-binding-form')).toBeVisible()
}

/** Answers the form's self-access-review check (fired on mount and on every namespace edit). */
async function grantAccess(
  page: Page,
  verb: string,
  namespace: string | null,
  allowed: boolean,
  reason: string | null = null,
): Promise<void> {
  await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
  await emit(
    page,
    batch([
      read(100, nsRoute('kind-a', ROLE_BINDINGS), {
        type: 'RbacSelfAccessUpdated',
        data: { results: [{ check: { verb, resource: 'rolebindings', namespace }, allowed, reason }] },
      }),
    ]),
  )
}

test.describe('US2 create RoleBinding', () => {
  test('T023: fill form, review diff, confirm, CreateResourceYaml sent; list updates only after refresh', async ({
    page,
  }) => {
    const form = await openCreateForm(page).then(() => page.getByTestId('rbac-binding-form'))

    await form.getByTestId('rbac-binding-namespace').fill('team-a')
    await grantAccess(page, 'create', 'team-a', true)

    await form.getByTestId('rbac-binding-roleref-name').fill('secret-reader')
    await form.getByTestId('rbac-subject-add').click()
    await form.getByTestId('rbac-subject-name').first().fill('alice')
    await form.getByTestId('rbac-binding-name').fill('view-secrets')

    const review = form.getByTestId('rbac-binding-review')
    await expect(review).toBeEnabled()
    await review.click()

    const diff = form.getByTestId('yaml-diff')
    await expect(diff).toBeVisible()
    await expect(diff).toContainText('kind: RoleBinding')
    await expect(diff).toContainText('secret-reader')
    await expect(diff).toContainText('alice')

    await clearSent(page)
    const id = await nextRequestId(page)
    await form.getByTestId('yaml-apply').click()

    const dialog = confirmDialog(page);
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['CreateResourceYaml']))
    const cmd = (await sent(page)).find((c) => c.type === 'CreateResourceYaml')
    expect(cmd).toMatchObject({
      type: 'CreateResourceYaml',
      data: {
        api_version: 'rbac.authorization.k8s.io/v1',
        kind: 'RoleBinding',
        namespace: 'team-a',
        name: 'view-secrets',
      },
    })

    await finish(page, id, true)
    await expect(form.getByTestId('rbac-binding-applied')).toBeVisible()

    // No optimistic list update: the row only appears once the list refresh event lands.
    await expectRowCount(page, 0, ROLE_BINDINGS)
    await emit(
      page,
      batch([
        list(101, nsRoute('kind-a', ROLE_BINDINGS), {
          type: 'RoleBindingsUpdated',
          data: [
            { name: 'view-secrets', namespace: 'team-a', role_ref: 'Role/secret-reader', subjects_count: 1 },
          ],
        }),
      ]),
    )
    await expectRowCount(page, 1, ROLE_BINDINGS)
    await expect(row(page, 'team-a/view-secrets', ROLE_BINDINGS)).toBeVisible()
  })

  test('cancel sends zero commands', async ({ page }) => {
    const form = await openCreateForm(page).then(() => page.getByTestId('rbac-binding-form'))
    await grantAccess(page, 'create', null, true)
    await form.getByTestId('rbac-binding-namespace').fill('team-a')
    await grantAccess(page, 'create', 'team-a', true)
    await form.getByTestId('rbac-binding-name').fill('view-secrets')

    await clearSent(page)
    await form.getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByTestId('rbac-binding-form')).toBeHidden()
    expect(await sent(page)).toEqual([])
  })

  test('create entry point is disabled with an explanation when self-access denies it', async ({
    page,
  }) => {
    const form = await openCreateForm(page).then(() => page.getByTestId('rbac-binding-form'))
    await grantAccess(page, 'create', null, false, 'rolebindings is forbidden: User cannot create in the given namespace')

    await expect(form.getByTestId('rbac-binding-create-disabled')).toBeVisible()
    await expect(form.getByTestId('rbac-binding-create-disabled')).toContainText('forbidden')
    await expect(form.getByTestId('rbac-binding-review')).toBeDisabled()

    await clearSent(page)
    await form.getByTestId('rbac-binding-review').click({ force: true })
    expect(await sent(page)).toEqual([])
  })
})

/**
 * US4 (011-rbac-oidc-administration, T037): the standalone "look up an identity's effective
 * namespace access" view (`ui/src/resources/namespaceAccess.tsx`) against the fake backend.
 * Backend resolution (`resolve_namespace_access`) is already tested in Rust — this only checks
 * that the frontend renders the `NamespaceAccessResolved` reply correctly and never fires a
 * write/mutating command.
 *
 * Navigation: `access/namespace-access` is registered in `ui/src/resources/nav.ts` under the
 * existing "Access Control" group as "Namespace Access" by a separate integration pass
 * (tasks.md T041/T042) that lands after this spec is written; `navigateTo` is the same helper
 * every other nav-driven spec in this suite uses, so this should work once that wiring lands.
 */
import { expect, type Page, test } from '@playwright/test'

import type { NamespaceAccessResolved } from '../../src/protocol/types'
import { batch, clusterRoute, read } from '../fixtures/builders'
import { emit, navigateTo, open, sent } from './helpers'

const SECTION = 'access/namespace-access'

async function openLookup(page: Page) {
  await open(page)
  await navigateTo(page, 'Namespace Access', 'Access Control')
  const view = page.getByTestId('rbac-namespace-access-lookup')
  await expect(view).toBeVisible()
  return view
}

async function submit(
  page: Page,
  view: ReturnType<typeof page.getByTestId>,
  subject: { kind: string; name: string; namespace?: string },
) {
  await view.getByTestId('rbac-namespace-access-kind').selectOption(subject.kind)
  await view.getByTestId('rbac-namespace-access-name').fill(subject.name)
  if (subject.namespace) {
    await view.getByTestId('rbac-namespace-access-subject-namespace').fill(subject.namespace)
  }
  await view.getByTestId('rbac-namespace-access-submit').click()
}

function resolved(overrides: Partial<NamespaceAccessResolved>): NamespaceAccessResolved {
  return {
    subject_kind: 'User',
    subject_name: 'jane',
    subject_namespace: null,
    cluster_wide: [],
    per_namespace: [],
    dangling_bindings: [],
    degraded_scopes: [],
    found_any: false,
    ...overrides,
  }
}

test.describe('US4 namespace access lookup', () => {
  test('resolved subject: cluster-wide grant rendered distinctly, per-namespace rows shown', async ({
    page,
  }) => {
    const view = await openLookup(page)
    await submit(page, view, { kind: 'User', name: 'jane' })

    await emit(
      page,
      batch([
        read(
          400,
          clusterRoute('kind-a', SECTION),
          {
            type: 'NamespaceAccessResolved',
            data: resolved({
              subject_name: 'jane',
              found_any: true,
              cluster_wide: [
                {
                  namespace: '',
                  verbs: ['get', 'list'],
                  resources: ['nodes'],
                  via: ['ClusterRoleBinding/node-viewers'],
                },
              ],
              per_namespace: [
                {
                  namespace: 'default',
                  verbs: ['get', 'list', 'watch'],
                  resources: ['pods'],
                  via: ['RoleBinding/pod-readers'],
                },
                {
                  namespace: 'kube-system',
                  verbs: ['get'],
                  resources: ['configmaps'],
                  via: ['RoleBinding/cm-readers'],
                },
              ],
            }),
          },
        ),
      ]),
    )

    const clusterWide = view.getByTestId('rbac-namespace-access-clusterwide')
    await expect(clusterWide).toBeVisible()
    await expect(clusterWide).toContainText('Cluster-wide')
    await expect(clusterWide).toContainText('node-viewers')
    // Cluster-wide entries must not render as if they were a namespace row (no "Namespace"
    // column/header in this block, and the empty `namespace` field is never rendered as a name).
    await expect(clusterWide.getByRole('columnheader', { name: 'Namespace' })).toHaveCount(0)

    await expect(view).toContainText('default')
    await expect(view).toContainText('kube-system')
    await expect(view).toContainText('pod-readers')
    await expect(view).toContainText('cm-readers')
    await expect(view.getByTestId('rbac-namespace-access-empty')).toHaveCount(0)
  })

  test('empty-result subject shows the empty state', async ({ page }) => {
    const view = await openLookup(page)
    await submit(page, view, { kind: 'Group', name: 'no-such-group' })

    await emit(
      page,
      batch([
        read(401, clusterRoute('kind-a', SECTION), {
          type: 'NamespaceAccessResolved',
          data: resolved({ subject_kind: 'Group', subject_name: 'no-such-group', found_any: false }),
        }),
      ]),
    )

    await expect(view.getByTestId('rbac-namespace-access-empty')).toBeVisible()
  })

  test('dangling bindings render alongside a resolved result without breaking the rest', async ({
    page,
  }) => {
    const view = await openLookup(page)
    await submit(page, view, { kind: 'User', name: 'jane' })

    await emit(
      page,
      batch([
        read(402, clusterRoute('kind-a', SECTION), {
          type: 'NamespaceAccessResolved',
          data: resolved({
            subject_name: 'jane',
            found_any: true,
            per_namespace: [
              {
                namespace: 'default',
                verbs: ['get'],
                resources: ['pods'],
                via: ['RoleBinding/pod-readers'],
              },
            ],
            dangling_bindings: ['RoleBinding/default/broken-binding'],
          }),
        }),
      ]),
    )

    const dangling = view.getByTestId('rbac-namespace-access-dangling')
    await expect(dangling).toBeVisible()
    await expect(dangling).toContainText('RoleBinding/default/broken-binding')
    // The rest of the resolved result still renders.
    await expect(view).toContainText('default')
    await expect(view).toContainText('pod-readers')
    await expect(view.getByTestId('rbac-namespace-access-empty')).toHaveCount(0)
  })

  test('empty subject name is rejected client-side without dispatching a command', async ({ page }) => {
    const view = await openLookup(page)
    await view.getByTestId('rbac-namespace-access-submit').click()

    await expect(view.getByTestId('rbac-namespace-access-error')).toBeVisible()
    expect((await sent(page)).map((c) => c.type)).not.toContain('ResolveNamespaceAccess')
  })

  test('never fires a write/mutating command', async ({ page }) => {
    const view = await openLookup(page)
    await submit(page, view, { kind: 'ServiceAccount', name: 'deployer', namespace: 'ci' })

    await emit(
      page,
      batch([
        read(403, clusterRoute('kind-a', SECTION), {
          type: 'NamespaceAccessResolved',
          data: resolved({
            subject_kind: 'ServiceAccount',
            subject_name: 'deployer',
            subject_namespace: 'ci',
            found_any: true,
            per_namespace: [
              { namespace: 'ci', verbs: ['create'], resources: ['pods'], via: ['RoleBinding/deployer'] },
            ],
          }),
        }),
      ]),
    )

    const types = (await sent(page)).map((c) => c.type)
    expect(types).toContain('ResolveNamespaceAccess')
    for (const writeType of ['ApplyResourceYaml', 'CreateResourceYaml', 'DeleteResource', 'DeleteResources']) {
      expect(types).not.toContain(writeType)
    }
  })
})

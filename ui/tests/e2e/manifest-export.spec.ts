/**
 * 040-manifest-export (T036): never blackbox the cluster. Inspector Copy / Download with clean
 * for re-apply, the row-menu and selection-bar export dialog (one multi-document bundle, read
 * fresh), the manifest shown in create flows before apply (RoleBinding form, kind create), and
 * the kubeconfig export with its redacted preview and confirm step. Every export is read-only:
 * no cluster-mutating command is sent.
 */
import { readFile } from 'node:fs/promises'

import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, list, nsRoute, read, stream, system } from '../fixtures/builders'
import { genDeployments } from '../fixtures/gen-pods'
import {
  check,
  clearSent,
  clusterMutations,
  confirmDialog,
  DEPLOYMENTS,
  emit,
  inspector,
  isPhone,
  navigateTo,
  open,
  prepareClipboard,
  readClipboard,
  row,
  sent,
  sentTypes,
  showList,
  table,
} from './helpers'

function deployment(i: number) {
  const d = genDeployments('kind-a')[i]
  if (!d) throw new Error('fixture: two deployments')
  return d
}
const D1 = deployment(0)
const D2 = deployment(1)
const key = (d: { namespace: string; name: string }) => `${d.namespace}/${d.name}`

async function openDeployments(page: Page): Promise<void> {
  await open(page)
  await showList(page, DEPLOYMENTS, 'Deployments')
  await expect(row(page, key(D1), DEPLOYMENTS)).toBeVisible()
}

test.describe('040 manifest export', () => {
  test('Inspector: copy clean, copy raw, download', async ({ page, context, browserName }) => {
    await prepareClipboard(page, context, browserName)
    await openDeployments(page)
    await row(page, key(D1), DEPLOYMENTS).click()
    const insp = inspector(page)
    await insp.getByRole('tab', { name: 'YAML' }).click()
    const bar = insp.getByTestId('manifest-actions')
    await expect(bar).toBeVisible()
    await expect(bar.getByTestId('manifest-clean')).toHaveAttribute('aria-checked', 'true')
    await clearSent(page)

    await bar.getByTestId('manifest-copy').click()
    await expect(page.getByTestId('manifest-copy-toast')).toBeVisible()
    await expect.poll(() => readClipboard(page, browserName)).toContain('kind: Deployment')
    const clean = await readClipboard(page, browserName)
    expect(clean).toContain(`name: ${D1.name}`)
    expect(clean).not.toMatch(/creationTimestamp|status:|deployment\.kubernetes\.io\/revision/)

    await bar.getByTestId('manifest-clean').click()
    await expect(bar.getByTestId('manifest-clean')).toHaveAttribute('aria-checked', 'false')
    await bar.getByTestId('manifest-copy').click()
    await expect.poll(() => readClipboard(page, browserName)).toContain('status:')
    expect(await readClipboard(page, browserName)).toContain('creationTimestamp')

    const [download] = await Promise.all([
      page.waitForEvent('download'),
      bar.getByTestId('manifest-download').click(),
    ])
    expect(download.suggestedFilename()).toBe(`deployment-${D1.namespace}-${D1.name}.yaml`)
    const path = await download.path()
    const body = await readFile(path, 'utf8')
    expect(body).toContain('kind: Deployment')
    expect(body).toContain('status:')

    expect(await clusterMutations(page)).toEqual([])
  })

  test('row menu: Export YAML reads fresh and shows one clean manifest', async ({ page }) => {
    await openDeployments(page)
    await clearSent(page)
    await row(page, key(D1), DEPLOYMENTS).click({ button: 'right' })
    const menu = page.getByTestId('row-action-menu')
    await expect(menu).toBeVisible()
    await menu.getByTestId('action-export-yaml').click()

    const dialog = page.getByTestId('manifest-export-dialog')
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('manifest-export-count')).toHaveText('1 of 1 objects')
    const fetch = (await sent(page)).find((c) => c.type === 'FetchManifests')
    expect(fetch).toMatchObject({
      type: 'FetchManifests',
      data: {
        objects: [
          { api_version: 'apps/v1', kind: 'Deployment', namespace: D1.namespace, name: D1.name },
        ],
      },
    })
    const yaml = dialog.getByTestId('manifest-export-yaml')
    await expect(yaml).toContainText(`name: ${D1.name}`)
    await expect(yaml).not.toContainText('status:')
    await dialog.getByTestId('manifest-clean').click()
    await expect(yaml).toContainText('status:')
    await page.keyboard.press('Escape')
    await expect(dialog).toBeHidden()
    expect(await clusterMutations(page)).toEqual([])
  })

  test('bulk export: a multi-selection downloads as one multi-document YAML', async ({ page }) => {
    await openDeployments(page)
    await check(page, key(D1), DEPLOYMENTS)
    await check(page, key(D2), DEPLOYMENTS)
    await clearSent(page)
    await page.getByTestId('bulk-export').click()

    const dialog = page.getByTestId('manifest-export-dialog')
    await expect(dialog.getByTestId('manifest-export-count')).toHaveText('2 of 2 objects')
    const [download] = await Promise.all([
      page.waitForEvent('download'),
      dialog.getByTestId('manifest-download').click(),
    ])
    expect(download.suggestedFilename()).toBe('deployment-2-objects.yaml')
    const body = await readFile(await download.path(), 'utf8')
    const docs = body.trimEnd().split('\n---\n')
    expect(docs).toHaveLength(2)
    for (const d of [D1, D2]) expect(body).toContain(`name: ${d.name}`)
    expect(body).not.toContain('status:')
    expect(await sentTypes(page)).toContain('FetchManifests')
    expect(await clusterMutations(page)).toEqual([])
  })

  test('RoleBinding form: the manifest is shown with Copy / Download before apply', async ({
    page,
    context,
    browserName,
  }) => {
    await prepareClipboard(page, context, browserName)
    await open(page)
    await navigateTo(page, 'Role Bindings', 'Access Control')
    await expect.poll(() => sentTypes(page)).toContain('RefreshRoleBindings')
    await emit(
      page,
      batch([
        list(1, nsRoute('kind-a', 'access/rolebindings'), {
          type: 'RoleBindingsUpdated',
          data: [],
        }),
      ]),
    )
    await expect(table(page, 'access/rolebindings')).toBeVisible()
    await page.getByTestId('rbac-binding-create-open').click()
    const form = page.getByTestId('rbac-binding-form')
    await expect(form).toBeVisible()
    await form.getByTestId('rbac-binding-namespace').fill('team-a')
    await expect.poll(() => sentTypes(page)).toContain('RefreshRbacSelfAccess')
    await emit(
      page,
      batch([
        read(100, nsRoute('kind-a', 'access/rolebindings'), {
          type: 'RbacSelfAccessUpdated',
          data: {
            results: [
              {
                check: {
                  verb: 'create',
                  resource: 'rolebindings',
                  namespace: 'team-a',
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
    await form.getByTestId('rbac-binding-roleref-name').fill('secret-reader')
    await form.getByTestId('rbac-subject-add').click()
    await form.getByTestId('rbac-subject-name').first().fill('alice')
    await form.getByTestId('rbac-binding-name').fill('view-secrets')
    await form.getByTestId('rbac-binding-review').click()

    await form.getByTestId('rbac-binding-view-manifest').click()
    const manifest = form.getByTestId('rbac-binding-manifest')
    await expect(manifest).toContainText('kind: RoleBinding')
    await expect(manifest).toContainText('secret-reader')
    await clearSent(page)
    await form.getByTestId('manifest-copy').click()
    await expect.poll(() => readClipboard(page, browserName)).toContain('kind: RoleBinding')
    expect(await readClipboard(page, browserName)).toContain('alice')
    const [download] = await Promise.all([
      page.waitForEvent('download'),
      form.getByTestId('manifest-download').click(),
    ])
    expect(download.suggestedFilename()).toBe('rolebinding-team-a-view-secrets.yaml')
    // Copy / Download never apply anything.
    expect(await clusterMutations(page)).toEqual([])
  })

  test('kind create: the cluster config is shown before create', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Cluster Settings', 'Team')
    await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
    await emit(
      page,
      batch([
        system(1, {
          type: 'ClusterSettingsLoaded',
          data: {
            context: 'kind-a',
            settings: {
              schema: 1,
              metricsSource: 'none',
              prometheusPath: '/api/v1',
              https: false,
              pathPrefix: '',
              hiddenMetrics: [],
              orgRevKnown: null,
              fieldOrigin: {
                metricsSource: 'local',
                prometheusPath: 'local',
                https: 'local',
                pathPrefix: 'local',
                hiddenMetrics: 'local',
              },
            },
          },
        }),
      ]),
    )
    const panel = page.getByTestId('kind-clusters')
    await expect(panel).toBeVisible()
    await emit(
      page,
      batch([
        stream(2, globalRoute(), {
          type: 'KindPrerequisitesUpdated',
          data: {
            prerequisites: {
              kind_installed: true,
              kind_version: 'kind v0.23.0',
              runtime_installed: true,
              runtime_kind: 'docker',
              runtime_running: true,
              ready: true,
            },
          },
        }),
        stream(3, globalRoute(), { type: 'KindClustersUpdated', data: { clusters: [] } }),
      ]),
    )
    await panel.getByTestId('kind-name-input').fill('demo')
    await clearSent(page)
    await panel.getByTestId('kind-create').click()
    const dialog = confirmDialog(page)
    await expect(dialog.getByTestId('kind-config-preview')).toContainText('kind: Cluster')
    await expect(dialog.getByTestId('kind-config-preview')).toContainText('role: control-plane')
    const [download] = await Promise.all([
      page.waitForEvent('download'),
      dialog.getByTestId('kind-config-download').click(),
    ])
    expect(download.suggestedFilename()).toBe('kind-demo.yaml')
    expect(await sentTypes(page)).not.toContain('CreateKindCluster')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    expect(await sentTypes(page)).not.toContain('CreateKindCluster')
  })

  test('kubeconfig export: redacted preview, credential flag, confirm before write', async ({
    page,
  }, info) => {
    await open(page)
    await clearSent(page)
    if (isPhone(info)) await page.getByTestId('scope-chip').click()
    await page.getByTestId('kubeconfig-export-open').click()

    const dialog = page.getByTestId('kubeconfig-export-dialog')
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('kubeconfig-credential')).toContainText('Bearer token')
    await expect(dialog.getByTestId('kubeconfig-embedded')).toContainText('token')
    await expect(dialog.getByTestId('kubeconfig-preview')).toContainText('token: <redacted>')
    await expect(dialog.getByTestId('kubeconfig-path')).toHaveValue(
      '/home/fathom/kind-a.kubeconfig',
    )
    // FR-026: the kubeconfig is never offered for the clipboard.
    await expect(dialog.getByTestId('manifest-copy')).toHaveCount(0)

    // Switching to the exec context: no embedded secret, the plugin is kept as-is.
    await dialog.getByTestId('kubeconfig-context').selectOption('kind-b')
    await expect(dialog.getByTestId('kubeconfig-credential')).toContainText(
      'Exec plugin (kubelogin)',
    )
    await expect(dialog.getByTestId('kubeconfig-embedded')).toHaveCount(0)
    await expect(dialog.getByTestId('kubeconfig-warning')).toContainText('kubelogin')
    await dialog.getByTestId('kubeconfig-context').selectOption('kind-a')
    await expect(dialog.getByTestId('kubeconfig-credential')).toContainText('Bearer token')

    await dialog.getByTestId('kubeconfig-include-credentials').click()
    await dialog.getByTestId('kubeconfig-export').click()
    const confirm = confirmDialog(page)
    await expect(confirm).toContainText('kind-a')
    await expect(confirm.getByTestId('kubeconfig-confirm-detail')).toContainText(
      '/home/fathom/kind-a.kubeconfig',
    )
    await expect(confirm.getByTestId('kubeconfig-confirm-detail')).toContainText(
      'Includes embedded credentials (token)',
    )
    expect(await sentTypes(page)).not.toContain('ExportKubeconfig')

    // Cancel writes nothing; confirming writes once.
    await confirm.getByRole('button', { name: 'Cancel' }).click()
    expect(await sentTypes(page)).not.toContain('ExportKubeconfig')
    await dialog.getByTestId('kubeconfig-export').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect(dialog.getByTestId('kubeconfig-result')).toHaveAttribute('data-ok', 'true')
    await expect(dialog.getByTestId('kubeconfig-result')).toContainText(
      'Wrote /home/fathom/kind-a.kubeconfig (0600) with embedded credentials',
    )
    const writes = (await sent(page)).filter((c) => c.type === 'ExportKubeconfig')
    expect(writes).toEqual([
      {
        type: 'ExportKubeconfig',
        data: {
          context: 'kind-a',
          path: '/home/fathom/kind-a.kubeconfig',
          include_embedded_credentials: true,
          overwrite: false,
        },
      },
    ])
    expect(await clusterMutations(page)).toEqual([])
  })
})

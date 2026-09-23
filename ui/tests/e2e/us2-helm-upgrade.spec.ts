/**
 * T023 (006-helm-mutate US2): the Upgrade tab's values-edit → preview-diff → confirm flow,
 * including a `helm upgrade --dry-run` failure (invalid values) surfacing the backend's error
 * verbatim, and the GitOps-managed-release warning banner appearing both before and after
 * confirming (never silently swallowed by the confirm step).
 */
import { expect, test, type Page } from '@playwright/test'

import type { GitOpsOriginRef, HelmReleaseDetails, HelmReleaseSummary } from '../../src/protocol/types'
import { batch, envelope, list, nsRoute, read } from '../fixtures/builders'
import { confirmDialog, emit, inspector, navigateTo, open, row, sent, sentTypes } from './helpers'

const HELM = 'helm/releases'

const REDIS: HelmReleaseSummary = {
  name: 'redis',
  namespace: 'cache',
  chart: 'redis',
  chart_version: '18.6.1',
  app_version: '7.2.4',
  revision: 3,
  status: 'Deployed' as const,
  updated: '2024-01-15T10:00:00Z',
  degraded: false,
}

const REDIS_DETAILS: HelmReleaseDetails = {
  name: REDIS.name,
  namespace: REDIS.namespace,
  chart: REDIS.chart,
  chart_version: REDIS.chart_version,
  app_version: REDIS.app_version,
  revision: REDIS.revision,
  status: REDIS.status,
  updated: REDIS.updated,
  description: 'Upgrade complete',
  values_user_supplied: 'replicaCount: 3',
  values_effective: 'replicaCount: 3\nimage:\n  repository: bitnami/redis\n  tag: 7.2.4',
  manifest: 'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis',
  revisions: [
    {
      revision: 3,
      status: 'Deployed',
      description: 'Upgrade complete',
      updated: REDIS.updated,
      app_version: '7.2.4',
      chart_version: '18.6.1',
    },
  ],
  origin: null,
  related: [],
  degraded: false,
}

const FLUX_ORIGIN: GitOpsOriginRef = {
  controller: 'Flux',
  section: 'flux/helm-releases',
  kind: 'HelmRelease',
  namespace: 'flux-system',
  name: 'redis',
  ready: 'Ready',
  message: null,
}

async function selectRedis(page: Page): Promise<void> {
  await open(page)
  await navigateTo(page, 'Helm Releases', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshHelmReleases')
  await emit(page, batch([list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [REDIS] })]))
  await row(page, `${REDIS.namespace}/${REDIS.name}`, HELM).click()
  await expect
    .poll(() => sent(page))
    .toEqual(
      expect.arrayContaining([
        { type: 'SelectHelmRelease', data: { namespace: REDIS.namespace, name: REDIS.name } },
      ]),
    )
  await emit(
    page,
    batch([
      read(2, nsRoute('kind-a', HELM, REDIS.namespace), {
        type: 'HelmReleaseDetailsUpdated',
        data: REDIS_DETAILS,
      }),
    ]),
  )
  const insp = inspector(page)
  await insp.getByRole('tab', { name: 'Upgrade' }).click()
}

test.describe('US2 Helm upgrade', () => {
  test('T023: values edit → preview → confirm applies the upgrade', async ({ page }) => {
    await selectRedis(page);
    const insp = inspector(page)
    const values = insp.getByTestId('helm-upgrade-values')
    await expect(values).toHaveValue('replicaCount: 3')
    await values.fill('replicaCount: 5')

    await insp.getByTestId('helm-upgrade-preview').click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'PreviewHelmUpgrade',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              values_yaml: 'replicaCount: 5',
              chart_version: null,
            },
          },
        ]),
      )

    await emit(
      page,
      batch([
        envelope(3, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: {
            type: 'HelmMutationPreviewReady',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              action: 'upgrade',
              base_revision: 3,
              current_manifest: REDIS_DETAILS.manifest,
              target_manifest: 'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis\n  replicas: 5',
              target_revision: null,
              target_chart_version: null,
              gitops_warning: null,
            },
          },
        }),
      ]),
    )

    const diff = insp.getByTestId('yaml-diff')
    await expect(diff).toBeVisible()
    await expect(diff.getByTestId('diff-added')).toContainText('replicas: 5')

    await insp.getByTestId('helm-upgrade-apply').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()
    await expect(dialog).toBeHidden()

    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'ApplyHelmUpgrade',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              values_yaml: 'replicaCount: 5',
              chart_version: null,
              base_revision: 3,
            },
          },
        ]),
      )

    await emit(
      page,
      batch([
        envelope(4, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: {
            type: 'HelmMutationApplied',
            data: { namespace: REDIS.namespace, release: REDIS.name, action: 'upgrade', new_revision: 4 },
          },
        }),
      ]),
    )
    await expect(insp.getByTestId('helm-upgrade-applied')).toBeVisible()
  })

  test('T023: an invalid-values preview failure surfaces the backend error verbatim', async ({ page }) => {
    await selectRedis(page)
    const insp = inspector(page)
    await insp.getByTestId('helm-upgrade-values').fill('not: [valid')
    await insp.getByTestId('helm-upgrade-preview').click()
    await expect.poll(() => sentTypes(page)).toContain('PreviewHelmUpgrade')

    await emit(
      page,
      batch([
        envelope(3, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: {
            type: 'HelmMutationFailed',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              action: 'upgrade',
              stage: 'preview',
              message: 'Error: YAML parse error on values.yaml: line 1: could not find expected \':\'',
            },
          },
        }),
      ]),
    )

    await expect(insp.getByTestId('helm-upgrade-error')).toContainText('YAML parse error')
    // No confirm dialog, and nothing was applied: a failed preview never reaches Apply.
    await expect(insp.getByTestId('helm-upgrade-apply')).toHaveCount(0)
  })

  test('T023: the GitOps-managed warning appears before and after confirming', async ({ page }) => {
    await selectRedis(page)
    const insp = inspector(page)
    await insp.getByTestId('helm-upgrade-preview').click()
    await expect.poll(() => sentTypes(page)).toContain('PreviewHelmUpgrade')

    await emit(
      page,
      batch([
        envelope(3, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: {
            type: 'HelmMutationPreviewReady',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              action: 'upgrade',
              base_revision: 3,
              current_manifest: REDIS_DETAILS.manifest,
              target_manifest: REDIS_DETAILS.manifest,
              target_revision: null,
              target_chart_version: null,
              gitops_warning: FLUX_ORIGIN,
            },
          },
        }),
      ]),
    )

    // Case 1: the warning is visible on the preview itself, before any confirmation.
    await expect(insp.getByTestId('helm-upgrade-gitops-warning')).toBeVisible()
    await expect(insp.getByTestId('helm-upgrade-gitops-warning')).toContainText('Flux')

    // Case 2: it is repeated in the confirm dialog itself, not just the panel behind it.
    await insp.getByTestId('helm-upgrade-apply').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText('Flux')
  })
})

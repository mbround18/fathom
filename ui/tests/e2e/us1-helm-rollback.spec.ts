/**
 * 006-helm-mutate T013 (US1 "roll back a release to a prior revision"): drives the rollback flow
 * end-to-end against the fake backend by injecting `HelmMutationPreviewReady`/
 * `HelmMutationApplied` events and asserting `PreviewHelmRollback`/`ApplyHelmRollback` commands
 * are emitted, covering both the GitOps-warning-present (Flux-owned fixture release, FR-009) and
 * GitOps-warning-absent (bare-release fixture, FR-010) cases.
 */
import { expect, test, type Page } from '@playwright/test'

import type {
  HelmMutationApplied,
  HelmMutationPreviewReady,
  HelmReleaseDetails,
  HelmReleaseSummary,
} from '../../src/protocol/types'
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
  status: 'Deployed',
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
  values_effective: 'replicaCount: 3',
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
    {
      revision: 2,
      status: 'Superseded',
      description: 'Upgrade complete',
      updated: '2024-01-10T10:00:00Z',
      app_version: '7.2.0',
      chart_version: '18.5.0',
    },
    {
      revision: 1,
      status: 'Superseded',
      description: 'Install complete',
      updated: '2024-01-01T10:00:00Z',
      app_version: '7.1.0',
      chart_version: '18.0.0',
    },
  ],
  origin: null,
  related: [],
  degraded: false,
}

const REDIS_DETAILS_FLUX: HelmReleaseDetails = {
  ...REDIS_DETAILS,
  origin: {
    section: 'flux/helmreleases',
    controller: 'Flux',
    kind: 'HelmRelease',
    namespace: REDIS.namespace,
    name: 'redis',
    ready: 'Ready',
    message: 'Release reconciled successfully',
  },
}

async function selectRelease(page: Page, details: HelmReleaseDetails): Promise<void> {
  await open(page)
  await navigateTo(page, 'Helm Releases', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshHelmReleases')
  await emit(
    page,
    batch([list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [REDIS] })]),
  )
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
        data: details,
      }),
    ]),
  )
  await expect(inspector(page)).toContainText('#2')
}

test.describe('US1 Helm rollback', () => {
  test('T013: rollback without GitOps origin previews, confirms (no warning), and applies', async ({
    page,
  }) => {
    await selectRelease(page, REDIS_DETAILS)
    const insp = inspector(page)

    await insp.getByTestId('helm-rollback-2').click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'PreviewHelmRollback',
            data: { namespace: REDIS.namespace, release: REDIS.name, target_revision: 2 },
          },
        ]),
      )

    const preview: HelmMutationPreviewReady = {
      namespace: REDIS.namespace,
      release: REDIS.name,
      action: 'rollback',
      base_revision: 3,
      current_manifest:
        'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis\n  labels:\n    v: "3"',
      target_manifest:
        'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis\n  labels:\n    v: "2"',
      target_revision: 2,
      target_chart_version: null,
      gitops_warning: null,
    }
    await emit(
      page,
      batch([
        envelope(3, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: { type: 'HelmMutationPreviewReady', data: preview },
        }),
      ]),
    )

    await expect(insp.getByTestId('yaml-diff')).toBeVisible()
    await expect(insp.getByTestId('helm-rollback-gitops-warning')).toHaveCount(0)

    await insp.getByTestId('helm-rollback-confirm-open').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog).not.toContainText('Managed by')
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'ApplyHelmRollback',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              target_revision: 2,
              base_revision: 3,
            },
          },
        ]),
      )

    const applied: HelmMutationApplied = {
      namespace: REDIS.namespace,
      release: REDIS.name,
      action: 'rollback',
      new_revision: 4,
    }
    await emit(
      page,
      batch([
        envelope(4, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: { type: 'HelmMutationApplied', data: applied },
        }),
      ]),
    )

    await expect(insp).toContainText('Rolled back.')
  })

  test('T013: rollback on a Flux-owned release shows the GitOps warning before confirming', async ({
    page,
  }) => {
    await selectRelease(page, REDIS_DETAILS_FLUX)
    const insp = inspector(page)

    await insp.getByTestId('helm-rollback-1').click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'PreviewHelmRollback',
            data: { namespace: REDIS.namespace, release: REDIS.name, target_revision: 1 },
          },
        ]),
      )

    const preview: HelmMutationPreviewReady = {
      namespace: REDIS.namespace,
      release: REDIS.name,
      action: 'rollback',
      base_revision: 3,
      current_manifest:
        'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis\n  labels:\n    v: "3"',
      target_manifest:
        'apiVersion: v1\nkind: Service\nmetadata:\n  name: redis\n  labels:\n    v: "1"',
      target_revision: 1,
      target_chart_version: null,
      gitops_warning: {
        section: 'flux/helmreleases',
        controller: 'Flux',
        kind: 'HelmRelease',
        namespace: REDIS.namespace,
        name: 'redis',
        ready: 'Ready',
        message: 'Release reconciled successfully',
      },
    }
    await emit(
      page,
      batch([
        envelope(3, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'System',
          data: { type: 'HelmMutationPreviewReady', data: preview },
        }),
      ]),
    )

    const warning = insp.getByTestId('helm-rollback-gitops-warning')
    await expect(warning).toBeVisible()
    await expect(warning).toContainText('Flux')

    await insp.getByTestId('helm-rollback-confirm-open').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText('Managed by Flux')
    await dialog.getByTestId('confirm-submit').click()

    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'ApplyHelmRollback',
            data: {
              namespace: REDIS.namespace,
              release: REDIS.name,
              target_revision: 1,
              base_revision: 3,
            },
          },
        ]),
      )
  })
})

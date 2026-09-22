/**
 * T031 (US3 "uninstall a release"): the Inspector's Actions tab confirms before uninstalling
 * (no diff-preview step, per spec.md's US3 rationale — the confirm dialog is the only gate),
 * warns when the release is GitOps-managed, dispatches `UninstallHelmRelease` on confirm, and
 * removes the release from the list (closing the Inspector) once the backend reports
 * `HelmMutationApplied`. A failed uninstall surfaces the error verbatim and leaves the release
 * (and Inspector) in place.
 */
import { expect, test, type Page } from '@playwright/test'

import type { GitOpsOriginRef, HelmReleaseDetails, HelmReleaseSummary } from '../../src/protocol/types'
import { batch, list, nsRoute, read, system } from '../fixtures/builders'
import { emit, inspector, navigateTo, open, row, sent, sentTypes } from './helpers'

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

const ORIGIN: GitOpsOriginRef = {
  controller: 'Flux' as const,
  section: 'flux/helmreleases',
  kind: 'HelmRelease',
  namespace: 'cache',
  name: 'redis',
  ready: 'Ready' as const,
  message: null,
}

const MANAGED: HelmReleaseSummary = {
  name: 'grafana',
  namespace: 'monitoring',
  chart: 'grafana',
  chart_version: '7.0.0',
  app_version: '10.0.0',
  revision: 2,
  status: 'Deployed' as const,
  updated: '2024-01-15T10:00:00Z',
  degraded: false,
}

function details(r: HelmReleaseSummary, origin: GitOpsOriginRef | null): HelmReleaseDetails {
  return {
    name: r.name,
    namespace: r.namespace,
    chart: r.chart,
    chart_version: r.chart_version,
    app_version: r.app_version,
    revision: r.revision,
    status: r.status,
    updated: r.updated,
    description: 'Install complete',
    values_user_supplied: '',
    values_effective: '',
    manifest: '',
    revisions: [],
    origin,
    related: [],
    degraded: false,
  }
}

async function selectRelease(page: Page, r: HelmReleaseSummary): Promise<void> {
  await open(page)
  await navigateTo(page, 'Helm Releases', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshHelmReleases')
  await emit(page, batch([list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [r] })]))
  await row(page, `${r.namespace}/${r.name}`, HELM).click()
  await expect
    .poll(() => sent(page))
    .toEqual(
      expect.arrayContaining([
        { type: 'SelectHelmRelease', data: { namespace: r.namespace, name: r.name } },
      ]),
    )
}

async function openActionsTab(page: Page): Promise<void> {
  const insp = inspector(page)
  await insp.locator('[data-testid="helm-actions-tab"]').click()
}

test.describe('US3 Helm release uninstall', () => {
  test('T031: uninstall without GitOps ownership — confirm, apply, list + Inspector update', async ({
    page,
  }) => {
    await selectRelease(page, REDIS)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details(REDIS, null),
        }),
      ]),
    )

    await openActionsTab(page)
    const insp = inspector(page)
    await insp.locator('[data-testid="helm-release-uninstall"]').click()

    const dialog = page.locator('[data-testid="confirm-dialog"]')
    await expect(dialog).toBeVisible()
    await expect(dialog.locator('[data-testid="helm-uninstall-gitops-warning"]')).toHaveCount(0)

    await dialog.locator('[data-testid="confirm-submit"]').click()

    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'UninstallHelmRelease',
            data: { namespace: REDIS.namespace, release: REDIS.name, base_revision: REDIS.revision },
          },
        ]),
      )

    await emit(
      page,
      batch([
        system(3, {
          type: 'HelmMutationApplied',
          data: {
            namespace: REDIS.namespace,
            release: REDIS.name,
            action: 'uninstall',
            new_revision: null,
          },
        }),
      ]),
    )

    // The release is gone from the list, and the Inspector's target cleared (it falls back to
    // its "nothing selected" empty state rather than the pane itself disappearing).
    await expect(row(page, `${REDIS.namespace}/${REDIS.name}`, HELM)).toHaveCount(0)
    await expect(insp).toContainText('Nothing selected')
  })

  test('T031: uninstall of a GitOps-managed release shows the ownership warning in the confirm dialog', async ({
    page,
  }) => {
    await selectRelease(page, MANAGED)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, MANAGED.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details(MANAGED, ORIGIN),
        }),
      ]),
    )

    await openActionsTab(page)
    const insp = inspector(page)
    await insp.locator('[data-testid="helm-release-uninstall"]').click()

    const dialog = page.locator('[data-testid="confirm-dialog"]')
    await expect(dialog).toBeVisible()
    await expect(dialog.locator('[data-testid="helm-uninstall-gitops-warning"]')).toBeVisible()

    await dialog.locator('[data-testid="confirm-submit"]').click()

    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'UninstallHelmRelease',
            data: {
              namespace: MANAGED.namespace,
              release: MANAGED.name,
              base_revision: MANAGED.revision,
            },
          },
        ]),
      )
  })

  test('T031: a failed uninstall surfaces the error and leaves the release + Inspector in place', async ({
    page,
  }) => {
    await selectRelease(page, REDIS)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details(REDIS, null),
        }),
      ]),
    )

    await openActionsTab(page)
    const insp = inspector(page)
    await insp.locator('[data-testid="helm-release-uninstall"]').click()
    await page.locator('[data-testid="confirm-dialog"] [data-testid="confirm-submit"]').click()

    await expect.poll(() => sentTypes(page)).toContain('UninstallHelmRelease')

    await emit(
      page,
      batch([
        system(3, {
          type: 'HelmMutationFailed',
          data: {
            namespace: REDIS.namespace,
            release: REDIS.name,
            action: 'uninstall',
            stage: 'apply',
            message: 'helm uninstall failed: release "redis" not found',
          },
        }),
      ]),
    )

    await expect(insp.locator('[data-testid="helm-uninstall-error"]')).toContainText(
      'release "redis" not found',
    )
    // Still selected: a failed uninstall does not clear the Inspector's target (only
    // `HelmMutationApplied` does), so it keeps showing this release rather than falling back to
    // the "Nothing selected" empty state.
    await expect(insp).not.toContainText('Nothing selected')
  })
})

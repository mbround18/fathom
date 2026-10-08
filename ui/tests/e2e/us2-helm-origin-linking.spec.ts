/**
 * T031 (US2 "trace release to GitOps origin"): a Helm release's inspector shows its GitOps
 * origin link when Flux ownership labels are present, omits it when absent, and clicking the
 * link opens the origin's own inspector (Ready/Failed state + message), including the explicit
 * not-found state for a deleted origin (constitution: never a blank/frozen panel).
 */
import { expect, test, type Page } from '@playwright/test'

import type { HelmReleaseDetails } from '../../src/protocol/types'
import { batch, list, nsRoute, read } from '../fixtures/builders'
import { emit, inspector, navigateTo, open, openedInspector, row, sent, sentTypes } from './helpers'
import { widthClass } from './layout-helpers'

/** Widths where slots 2 and up are cascade columns (044 FR-005), so both panes stay on screen. */
const CASCADE_WIDTHS = ['wide', 'ultra', 'ultra-plus']

const HELM = 'helm/releases'
const FLUX_RESOURCE = 'integrations/flux-resource'

const REDIS = {
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

const BARE = {
  name: 'bare-nginx',
  namespace: 'default',
  chart: 'nginx',
  chart_version: '15.0.0',
  app_version: '1.25.0',
  revision: 1,
  status: 'Deployed' as const,
  updated: '2024-03-01T09:00:00Z',
  degraded: false,
}

function details(overrides: Partial<HelmReleaseDetails> = {}): HelmReleaseDetails {
  return {
    name: REDIS.name,
    namespace: REDIS.namespace,
    chart: REDIS.chart,
    chart_version: REDIS.chart_version,
    app_version: REDIS.app_version,
    revision: REDIS.revision,
    status: REDIS.status,
    updated: REDIS.updated,
    description: null,
    values_user_supplied: 'replicaCount: 3',
    values_effective: 'replicaCount: 3\nimage:\n  tag: 7.2.4',
    manifest: 'apiVersion: v1\nkind: Service',
    revisions: [],
    origin: null,
    related: [],
    degraded: false,
    ...overrides,
  }
}

const ORIGIN = {
  controller: 'Flux' as const,
  section: FLUX_RESOURCE,
  kind: 'HelmRelease',
  namespace: 'flux-system',
  name: 'redis',
  ready: 'Ready' as const,
  message: 'Release reconciliation succeeded',
}

async function selectRelease(page: Page, r: typeof REDIS): Promise<void> {
  await open(page)
  await navigateTo(page, 'Helm Releases', 'Overview')
  await expect.poll(() => sentTypes(page)).toContain('RefreshHelmReleases')
  await emit(
    page,
    batch([list(1, nsRoute('kind-a', HELM), { type: 'HelmReleasesUpdated', data: [r] })]),
  )
  await row(page, `${r.namespace}/${r.name}`, HELM).click()
  await expect
    .poll(() => sent(page))
    .toEqual(
      expect.arrayContaining([
        { type: 'SelectHelmRelease', data: { namespace: r.namespace, name: r.name } },
      ]),
    )
}

test.describe('US2 Helm release GitOps origin linking', () => {
  test('T031: shows the origin link with Ready state, and follows it to the Flux resource', async ({
    page,
  }) => {
    await selectRelease(page, REDIS)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details({ origin: ORIGIN }),
        }),
      ]),
    )

    const insp = inspector(page)
    await expect(insp).toContainText('flux-system/redis')
    await expect(insp).toContainText('Release reconciliation succeeded')

    await insp.getByText('flux-system/redis').click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'SelectFluxResource',
            data: { kind: 'HelmRelease', namespace: 'flux-system', name: 'redis' },
          },
        ]),
      )

    await emit(
      page,
      batch([
        read(3, nsRoute('kind-a', FLUX_RESOURCE, 'flux-system'), {
          type: 'FluxResourceDetailsUpdated',
          data: {
            kind: 'HelmRelease',
            name: 'redis',
            namespace: 'flux-system',
            ready: 'Ready',
            message: 'Release reconciliation succeeded',
            revision: 'redis@18.6.1',
            source_ref: 'HelmRepository/bitnami',
            suspended: false,
            interval: '5m',
            last_transition: '2024-03-01T09:00:00Z',
            managed: [],
          },
        }),
      ]),
    )
    // 044 (FR-005): at wide and above the followed origin opens the next cascade column.
    await expect(openedInspector(page)).toContainText('HelmRelease')
    await expect(openedInspector(page)).toContainText('bitnami')
  })

  /**
   * The common real-world arrangement: the `HelmRelease` CR lives in the release's own namespace
   * under the release's own name (`origin_from_labels` falls back to that namespace when the Flux
   * namespace label is absent). Both objects then key identically unless `detailKind` namespaces
   * the origin, and the Flux reply blanks the release column it was opened from — invisible before
   * 044 replaced the one inspector, plainly visible once both columns are on screen.
   */
  test('T031: a same-namespace origin does not blank the release column it was opened from', async ({
    page,
  }) => {
    const sameNs = { ...ORIGIN, namespace: REDIS.namespace }
    await selectRelease(page, REDIS)
    // Below wide the origin retargets the single inspector, so the release pane is *meant* to go.
    test.skip(!CASCADE_WIDTHS.includes((await widthClass(page)) ?? ''), 'wide and above only')
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details({ origin: sameNs }),
        }),
      ]),
    )

    const insp = inspector(page)
    await expect(insp).toContainText('replicaCount: 3')
    await insp.getByText(`${REDIS.namespace}/redis`).click()
    await emit(
      page,
      batch([
        read(3, nsRoute('kind-a', FLUX_RESOURCE, REDIS.namespace), {
          type: 'FluxResourceDetailsUpdated',
          data: {
            kind: 'HelmRelease',
            name: 'redis',
            namespace: REDIS.namespace,
            ready: 'Ready',
            message: 'Release reconciliation succeeded',
            revision: 'redis@18.6.1',
            source_ref: 'HelmRepository/bitnami',
            suspended: false,
            interval: '5m',
            last_transition: '2024-03-01T09:00:00Z',
            managed: [],
          },
        }),
      ]),
    )

    await expect(openedInspector(page)).toContainText('bitnami')
    // The release's own detail-only sections are still rendered, not back to skeletons.
    await expect(insp).toContainText('replicaCount: 3')
    await expect(insp).toContainText('kind: Service')
    await expect(insp.getByRole('status', { name: 'Loading' })).toHaveCount(0)
  })

  test('T031: omits the origin link for a bare Helm release with no Flux ownership labels', async ({
    page,
  }) => {
    await selectRelease(page, BARE)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, BARE.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details({
            name: BARE.name,
            namespace: BARE.namespace,
            chart: BARE.chart,
            chart_version: BARE.chart_version,
            app_version: BARE.app_version,
            revision: BARE.revision,
            origin: null,
          }),
        }),
      ]),
    )

    await expect(inspector(page)).toContainText('Not managed by a GitOps controller')
  })

  test('T031: a deleted origin resolves to an explicit not-found state, not a blank panel', async ({
    page,
  }) => {
    await selectRelease(page, REDIS)
    await emit(
      page,
      batch([
        read(2, nsRoute('kind-a', HELM, REDIS.namespace), {
          type: 'HelmReleaseDetailsUpdated',
          data: details({ origin: ORIGIN }),
        }),
      ]),
    )

    await inspector(page).getByText('flux-system/redis').click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'SelectFluxResource',
            data: { kind: 'HelmRelease', namespace: 'flux-system', name: 'redis' },
          },
        ]),
      )

    await emit(
      page,
      batch([
        read(3, nsRoute('kind-a', FLUX_RESOURCE, 'flux-system'), {
          type: 'FluxResourceNotFound',
          data: { kind: 'HelmRelease', namespace: 'flux-system', name: 'redis' },
        }),
      ]),
    )

    // 044 (FR-005): the not-found state belongs to the column the link opened.
    await expect(openedInspector(page)).toContainText('No longer exists')
    await expect(openedInspector(page)).toContainText('redis was deleted from the cluster.')
  })
})

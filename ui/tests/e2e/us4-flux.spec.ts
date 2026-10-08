/** US4 (T099): the Flux dashboard renders sources/kustomizations/helmreleases with status glyphs. */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { FluxOverview, FluxResourceSummary } from '../../src/protocol/types'
import { emit, expectNoHorizontalScroll, navigateTo, open, sentTypes } from './helpers'

function res(
  p: Partial<FluxResourceSummary> & { kind: string; name: string },
): FluxResourceSummary {
  return {
    namespace: 'flux-system',
    ready: 'Ready',
    message: 'Applied revision main@sha1:abc',
    revision: 'main@sha1:abc',
    source_ref: null,
    suspended: false,
    interval: '1m',
    last_transition: '2024-01-01T00:00:00Z',
    managed: [],
    ...p,
  }
}

const FLUX: FluxOverview = {
  installed: true,
  sources: [
    res({ kind: 'GitRepository', name: 'fleet' }),
    res({ kind: 'HelmRepository', name: 'bitnami', ready: 'Failed', message: 'auth failed' }),
  ],
  kustomizations: [
    res({ kind: 'Kustomization', name: 'apps', source_ref: 'GitRepository/fleet' }),
    res({ kind: 'Kustomization', name: 'infra', ready: 'Reconciling', message: 'in progress' }),
  ],
  helmreleases: [
    res({ kind: 'HelmRelease', name: 'redis', ready: 'Suspended', message: 'suspended' }),
    res({ kind: 'HelmRelease', name: 'nginx' }),
  ],
}

test.describe('US4 Flux dashboard', () => {
  test('waits for the scan, then shows tile counts and status-toned rows', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Flux', 'Integrations')
    await expect.poll(() => sentTypes(page)).toContain('RefreshFlux')
    await expect(page.locator('[data-pane-state="metrics-unavailable"]').first()).toContainText(
      'Waiting for the Flux scan',
    )

    await emit(page, batch([system(1, { type: 'FluxOverviewUpdated', data: FLUX })]))

    const pane = page.locator('[data-slot="pane"][data-section="integrations/flux"]').first()
    await expect(pane.getByText('1/2', { exact: true })).toHaveCount(3)
    for (const label of ['Sources', 'Kustomizations', 'HelmReleases']) {
      await expect(pane.getByText(label, { exact: true })).toBeVisible()
    }
    // 049 (US3): every resource is listed with its status glyph (tone dot), the ones needing
    // attention first within each kind group.
    const failed = pane.locator('[data-tone="err"]')
    await expect(failed).toHaveCount(1)
    await expect(failed).toContainText('HelmRepository flux-system/bitnami')
    await expect(pane.locator('[data-tone="warn"]')).toContainText(
      'Kustomization flux-system/infra',
    )
    await expect(pane.locator('[data-tone="muted"]')).toContainText('HelmRelease flux-system/redis')
    const rows = pane.getByTestId('flux-row')
    await expect(rows).toHaveCount(6)
    await expect(rows.nth(0)).toContainText('HelmRepository flux-system/bitnami')
    await expect(rows.nth(1)).toContainText('GitRepository flux-system/fleet')
    await expectNoHorizontalScroll(page)
  })

  test('says Flux is not installed when the CRDs are missing', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Flux', 'Integrations')
    await emit(
      page,
      batch([
        system(1, {
          type: 'FluxOverviewUpdated',
          data: { installed: false, sources: [], kustomizations: [], helmreleases: [] },
        }),
      ]),
    )
    await expect(page.locator('[data-pane-state="metrics-unavailable"]').first()).toContainText(
      'Flux GitOps Toolkit CRDs were not found',
    )
  })
})

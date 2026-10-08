/** Shared fixture for the 049 Flux verb specs: an overview the fake backend "scanned". */
import type { Page } from '@playwright/test'
import { expect } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { FluxOverview, FluxResourceSummary } from '../../src/protocol/types'
import { emit, navigateTo, open, openedInspector, sentTypes } from './helpers'

export function res(
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

export const FLUX: FluxOverview = {
  installed: true,
  sources: [
    res({ kind: 'GitRepository', name: 'fleet' }),
    res({ kind: 'HelmRepository', name: 'podinfo', namespace: 'fathom-dev' }),
  ],
  kustomizations: [
    res({
      kind: 'Kustomization',
      name: 'apps',
      source_ref: 'GitRepository/fleet',
      managed: ['Deployment default/web', 'Service default/web', 'ConfigMap default/web-env'],
    }),
    res({
      kind: 'Kustomization',
      name: 'infra',
      source_ref: 'GitRepository/fleet',
      suspended: true,
      ready: 'Suspended',
      message: 'suspended',
    }),
  ],
  helmreleases: [
    res({
      kind: 'HelmRelease',
      name: 'podinfo',
      namespace: 'fathom-dev',
      source_ref: 'HelmRepository/podinfo',
      managed: ['Helm release fathom-dev/podinfo'],
    }),
  ],
}

/** Open the Flux pane with the fixture scanned, and click one row into the inspector. */
export async function openFluxRow(page: Page, label: string): Promise<void> {
  await open(page)
  await navigateTo(page, 'Flux', 'Integrations')
  await expect.poll(() => sentTypes(page)).toContain('RefreshFlux')
  await emit(page, batch([system(1, { type: 'FluxOverviewUpdated', data: FLUX })]))
  const pane = page.locator('[data-slot="pane"][data-section="integrations/flux"]').first()
  await pane.getByTestId('flux-row').filter({ hasText: label }).first().click()
  await expect(openedInspector(page)).toContainText(label.split(' ').at(-1) ?? label)
}

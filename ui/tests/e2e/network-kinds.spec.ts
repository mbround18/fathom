/** Batch B (034): Endpoints, Ingresses, IngressClasses, NetworkPolicies list views. */
import { expect, test } from '@playwright/test'

import { batch, clusterRoute, list, nsRoute } from '../fixtures/builders'
import { ENDPOINTS, INGRESSES, INGRESS_CLASSES, NETPOLS } from '../fixtures/network'
import { emit, expectRowCount, navigateTo, open, row, sentTypes } from './helpers'

const CASES = [
  {
    label: 'Endpoints',
    section: 'network/endpoints',
    refresh: 'RefreshEndpoints',
    event: { type: 'EndpointsUpdated', data: ENDPOINTS },
    count: 1,
    key: 'default/web',
  },
  {
    label: 'Ingresses',
    section: 'network/ingress',
    refresh: 'RefreshIngresses',
    event: { type: 'IngressesUpdated', data: INGRESSES },
    count: 2,
    key: 'default/web',
  },
  {
    label: 'Ingress classes',
    section: 'network/ingressclasses',
    refresh: 'RefreshIngressClasses',
    event: { type: 'IngressClassesUpdated', data: INGRESS_CLASSES },
    count: 1,
    key: 'nginx',
  },
  {
    label: 'Network policies',
    section: 'network/networkpolicies',
    refresh: 'RefreshNetworkPolicies',
    event: { type: 'NetworkPoliciesUpdated', data: NETPOLS },
    count: 1,
    key: 'default/deny-all',
  },
] as const

for (const c of CASES) {
  test(`${c.label} lists rows`, async ({ page }) => {
    await open(page)
    await navigateTo(page, c.label, 'Network')
    await expect.poll(() => sentTypes(page)).toContain(c.refresh)
    await emit(
      page,
      batch([
        list(
          1,
          c.section === 'network/ingressclasses'
            ? clusterRoute('kind-a', c.section)
            : nsRoute('kind-a', c.section),
          c.event,
        ),
      ]),
    )
    await expectRowCount(page, c.count, c.section)
    await expect(row(page, c.key, c.section)).toBeVisible()
  })
}

/** 019: cluster version badge — server version, unknown state, and skew warning. */
import { expect, test } from '@playwright/test'

import {
  clusterVersionHealthy,
  clusterVersionSkewed,
  clusterVersionUnknown,
  federationContexts,
} from '../fixtures'
import { emit, open } from './helpers'

test.describe('019 cluster version badge', () => {
  test('shows server version, neutral unknown, and skew warning', async ({ page }) => {
    await open(page, 'manual')
    await emit(page, federationContexts)
    const badge = page.getByTestId('cluster-version-badge')

    await expect(badge).toContainText('Unknown')
    await expect(badge).toHaveAttribute('data-skew-detected', 'false')

    await emit(page, clusterVersionHealthy)
    await expect(badge).toHaveAttribute('data-server-version', 'v1.29.4')
    await expect(badge).toContainText('v1.29.4')
    await expect(page.getByTestId('cluster-version-skew-details')).toHaveCount(0)

    await emit(page, clusterVersionSkewed)
    await expect(badge).toHaveAttribute('data-skew-detected', 'true')
    await expect(page.getByTestId('cluster-version-skew-details')).toContainText('v1.28.9')

    await emit(page, clusterVersionHealthy)
    await expect(badge).toHaveAttribute('data-skew-detected', 'false')
    await expect(page.getByTestId('cluster-version-skew-details')).toHaveCount(0)

    await emit(page, clusterVersionUnknown)
    await expect(badge).toContainText('Unknown')
  })
})

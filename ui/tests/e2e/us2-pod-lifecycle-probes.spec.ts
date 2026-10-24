/**
 * T016 (US2): the Lifecycle section lists each container's configured probes (readiness,
 * liveness, startup) and flags the ones currently failing, correlated from the Pod's own Events
 * (FR-007). A container with no probes configured shows an explicit empty state.
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { podLifecycleDetail, podLifecycleEvents } from '../fixtures/batches'
import { emit, inspector, open, row, sentTypes, settleFakeReplies } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US2 pod lifecycle: probes', () => {
  test('probe config renders per container; liveness failure is flagged from a matching event', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')

    // The poll above proves `SelectPod` was *sent*, not answered. At ultra-plus the cascade mounts
    // six panes at once, so canned replies are still queued; drain them or one lands after the
    // injection below and takes the lifecycle detail back out.
    await settleFakeReplies(page)

    await emit(page, podLifecycleDetail('kind-a', DETAIL_POD))
    await emit(page, podLifecycleEvents(DETAIL_POD))

    const probes = insp.getByTestId('pod-lifecycle-probes')
    await expect(probes).toContainText('readiness')
    await expect(probes).toContainText('liveness')
    await expect(probes).toContainText('startup')

    // The app container is running but not ready -> its readiness probe is flagged.
    // Its Unhealthy/"Liveness probe failed" event -> its liveness probe is flagged too.
    const failing = probes.getByTestId('pod-lifecycle-probe-failing')
    await expect(failing.filter({ hasText: 'readiness' })).toHaveCount(1)
    await expect(failing.filter({ hasText: 'liveness' })).toHaveCount(1)
    // No startup-probe-failed event was injected -> not flagged.
    await expect(failing.filter({ hasText: 'startup' })).toHaveCount(0)

    // The sidecar and init containers have no probes configured -> an empty state for them.
    await expect(probes.getByTestId('pod-lifecycle-probes-empty').first()).toBeVisible()
  })
})

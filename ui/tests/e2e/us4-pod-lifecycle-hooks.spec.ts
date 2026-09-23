/**
 * T027 (US4): the Lifecycle section lists each container's configured lifecycle hooks
 * (postStart/preStop) and flags one as failed when a matching `FailedPostStartHook` /
 * `FailedPreStopHook` event mentions the container (FR-012, FR-013). A container with no hooks
 * configured shows an explicit empty state.
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { podLifecycleDetail, podLifecycleEvents } from '../fixtures/batches'
import { emit, inspector, open, row, sentTypes } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US4 pod lifecycle: hooks', () => {
  test('lifecycle hooks render per container; a matching FailedPostStartHook event flags postStart', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')

    await emit(page, podLifecycleDetail('kind-a', DETAIL_POD))
    await emit(page, podLifecycleEvents(DETAIL_POD))

    const hooks = insp.getByTestId('pod-lifecycle-hooks')
    await expect(hooks).toContainText('postStart')
    await expect(hooks).toContainText('preStop')

    const failed = hooks.getByTestId('pod-lifecycle-hook-failed')
    await expect(failed).toHaveCount(1)
    await expect(failed).toContainText('postStart')

    // The sidecar and init containers have no hooks configured -> an empty state for them.
    await expect(hooks.getByTestId('pod-lifecycle-hooks-empty').first()).toBeVisible()
  })
})

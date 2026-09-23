/**
 * T010 (US1): the Pod Inspector's Lifecycle section lists init containers separately from app
 * containers, with their terminal exit code, and an empty state when a pod has none (FR-001,
 * FR-002). "View logs" on an init container switches to the Logs tab and starts a log stream
 * scoped to that container, the same `StartPodLogs` command app-container logs use (FR-003).
 *
 * All new data-testids are prefixed `pod-lifecycle-` (isolation from 010-container-image-
 * inspection's Pod Inspector work on the same page).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { podLifecycleDetail, podLifecycleEvents } from '../fixtures/batches'
import { clearSent, emit, inspector, open, row, sent, sentTypes } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US1 pod lifecycle: init containers', () => {
  test('init containers list separately, with exit codes; empty state when none; view-logs sends StartPodLogs', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')

    // The default (non-lifecycle) fixture has no init containers -> the empty state shows.
    await expect(insp.getByTestId('pod-lifecycle-init-containers-empty')).toBeVisible()

    // Now the backend reports a richer pod: two init containers (one ok, one failed).
    await clearSent(page)
    await emit(page, podLifecycleDetail('kind-a', DETAIL_POD))
    await emit(page, podLifecycleEvents(DETAIL_POD))

    const initSection = insp.getByTestId('pod-lifecycle-init-containers')
    await expect(initSection.getByTestId('pod-lifecycle-init-containers-empty')).toHaveCount(0)

    const migrate = insp.getByTestId('pod-lifecycle-init-container-init-migrate')
    await expect(migrate).toContainText('init-migrate')
    await expect(migrate).toContainText('exit 0')

    const verify = insp.getByTestId('pod-lifecycle-init-container-init-verify')
    await expect(verify).toContainText('init-verify')
    await expect(verify).toContainText('exit 1')

    // App/sidecar containers are not listed among init containers.
    await expect(initSection).not.toContainText('sidecar')

    // "View logs" on an init container switches to Logs and starts a stream for it.
    await migrate.getByTestId('pod-lifecycle-view-logs').click()
    await expect(insp.getByRole('tab', { name: 'Logs' })).toHaveAttribute('aria-selected', 'true')
    await expect
      .poll(() => sentTypes(page))
      .toEqual(expect.arrayContaining(['StartPodLogs']))
    const starts = (await sent(page)).filter((c) => c.type === 'StartPodLogs')
    expect(starts.at(-1)).toMatchObject({
      type: 'StartPodLogs',
      data: { namespace: DETAIL_POD.namespace, name: DETAIL_POD.name, container: 'init-migrate' },
    })
  })
})

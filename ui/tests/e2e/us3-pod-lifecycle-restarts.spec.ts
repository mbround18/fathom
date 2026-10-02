/**
 * T022 (US3): the Lifecycle section shows each container's restart count and the reason/exit
 * code of its last termination (FR-009, FR-010), including the `restartPolicy: Never` edge case
 * where a container that never restarted still has termination info surfaced from `state` (no
 * `last_state`, since the kubelet never populates it when `restart_count === 0`).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { podLifecycleDetail, podLifecycleEvents } from '../fixtures/batches'
import { batch, nsRoute, read } from '../fixtures/builders'
import { podDetailsWithLifecycle } from '../fixtures/gen-pods'
import { emit, inspector, open, row, sentTypes, settleFakeReplies } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US3 pod lifecycle: restart history', () => {
  test('shows restart count and last termination reason/exit code per container', async ({
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

    const restarts = insp.getByTestId('pod-lifecycle-restarts')
    // The app container restarted 3 times, last via OOMKilled (exit 137).
    await expect(restarts).toContainText('restarts: 3')
    await expect(restarts).toContainText('OOMKilled')
    await expect(restarts).toContainText('exit 137')
    // The sidecar never restarted.
    await expect(restarts).toContainText('No restarts.')
  })

  test('restartPolicy: Never edge case — a container terminated with restart_count 0 still shows its exit reason', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')

    const detail = podDetailsWithLifecycle(DETAIL_POD)
    const neverRestarted = {
      ...detail,
      containers_lifecycle: [
        {
          name: 'batch-job',
          is_init: false,
          is_ephemeral: false,
          image: 'registry.example.com/batch:1.0.0',
          ready: false,
          state: {
            phase: 'terminated' as const,
            reason: 'Error',
            exit_code: 1,
            started_at: '2026-09-20T08:00:00Z',
            finished_at: '2026-09-20T08:00:05Z',
          },
          restart_count: 0,
          last_state: null,
          probes: [],
          hooks: [],
        },
      ],
    }
    // The poll above proves `SelectPod` was *sent*, not answered. At ultra-plus the cascade mounts
    // six panes at once, so canned replies are still queued; drain them or one lands after the
    // injection below and takes the lifecycle detail back out.
    await settleFakeReplies(page)

    await emit(
      page,
      batch([
        read(60, nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace), {
          type: 'PodDetailsUpdated',
          data: neverRestarted,
        }),
      ]),
    )

    const restarts = insp.getByTestId('pod-lifecycle-restarts')
    await expect(restarts).toContainText('restarts: 0')
    await expect(restarts).toContainText('Error')
    await expect(restarts).toContainText('exit 1')
  })
})

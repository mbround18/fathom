/**
 * T032 (US5): the Lifecycle section shows the Pod's own Events as an in-place timeline (FR-016),
 * still shows an event with a reason Fathom does not specifically recognize rather than hiding it
 * (FR-019), and updates live from a later `EventsUpdated` batch without leaving the section
 * (FR-017/FR-016 "no duplicate fetch" — driven by the same cluster-events list the Events tab
 * uses).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { podLifecycleDetail, podLifecycleEvents } from '../fixtures/batches'
import { batch, clusterRoute, list } from '../fixtures/builders'
import { podEventsWithLifecycle } from '../fixtures/gen-pods'
import { emit, inspector, open, row, sentTypes } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US5 pod lifecycle: events timeline', () => {
  test('shows the pod events timeline, including an unrecognized reason, and updates live', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')

    await emit(page, podLifecycleDetail('kind-a', DETAIL_POD))
    await emit(page, podLifecycleEvents(DETAIL_POD))

    const timeline = insp.getByTestId('pod-lifecycle-events-timeline')
    await expect(timeline).toContainText('Scheduled')
    await expect(timeline).toContainText('Unhealthy')
    await expect(timeline).toContainText('FailedPostStartHook')
    // FR-019: an event whose reason Fathom doesn't specifically recognize is still shown as-is.
    await expect(timeline).toContainText('SomeUnknownReason')

    // A later EventsUpdated batch (e.g. a fresh BackOff) reflects live, in place.
    const more = [
      ...podEventsWithLifecycle(DETAIL_POD),
      {
        namespace: DETAIL_POD.namespace,
        involved_object_kind: 'Pod' as const,
        involved_object_name: DETAIL_POD.name,
        name: `${DETAIL_POD.name}.late-backoff`,
        event_type: 'Warning' as const,
        reason: 'BackOff',
        message: 'Back-off restarting failed container',
        count: 1,
        first_timestamp: '2026-09-20T09:00:00Z',
        last_timestamp: '2026-09-20T09:00:00Z',
        source: 'kubelet',
      },
    ]
    await emit(
      page,
      batch([
        list(60, clusterRoute('kind-a', 'events'), {
          type: 'EventsUpdated',
          data: { events: more, total_count: more.length },
        }),
      ]),
    )
    await expect(timeline).toContainText('BackOff')
  })

  test('shows an explicit empty state when the pod has no events', async ({ page }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')

    await emit(
      page,
      batch([
        list(60, clusterRoute('kind-a', 'events'), {
          type: 'EventsUpdated',
          data: { events: [], total_count: 0 },
        }),
      ]),
    )
    await expect(insp.getByTestId('pod-lifecycle-events-timeline-empty')).toBeVisible()
  })
})

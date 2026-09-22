/**
 * T011 (US1 Phase 3): the Logs tab against the fake backend. Selecting a pod and opening Logs
 * sends `StartPodLogs`, injected `PodLogsChunk` events render as lines, and leaving the tab
 * (closing the inspector) sends `StopPodLogs` (FR-002, FR-004, FR-005, FR-006).
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { batch, nsRoute, stream } from '../fixtures/builders'
import { clearSent, emit, inspector, open, row, sent } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US1 Logs tab', () => {
  test('opening Logs sends StartPodLogs, chunks render, closing sends StopPodLogs', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(insp.getByRole('heading', { name: DETAIL_POD.name })).toBeVisible()

    await clearSent(page)
    await insp.getByRole('tab', { name: 'Logs' }).click()
    await expect(insp.getByRole('tab', { name: 'Logs' })).toHaveAttribute('aria-selected', 'true')

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['StartPodLogs']))
    const start = (await sent(page)).find((c) => c.type === 'StartPodLogs')
    expect(start).toMatchObject({
      type: 'StartPodLogs',
      data: { namespace: DETAIL_POD.namespace, name: DETAIL_POD.name },
    })

    const route = nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace)
    await emit(
      page,
      batch([
        stream(200, route, {
          type: 'PodLogsChunk',
          data: { stream_id: 1, lines: ['booting…', 'listening on :8080'] },
        }),
      ]),
    )
    const scroll = insp.getByTestId('pod-logs-scroll')
    await expect(scroll).toContainText('booting…')
    await expect(scroll).toContainText('listening on :8080')

    // FR-006: a StreamClosed shows a distinct "stream ended" indicator, not a blank pane.
    await emit(
      page,
      batch([stream(201, route, { type: 'StreamClosed', data: { stream_id: 1, reason: 'EOF' } })]),
    )
    await expect(insp.locator('[data-pane-state="stream-ended"]')).toBeVisible()
    await expect(insp.locator('[data-pane-state="stream-ended"]')).toContainText('EOF')

    // Leaving the tab (closing the inspector) stops the stream (FR-005).
    await clearSent(page)
    await page.keyboard.press('Escape')
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toEqual(expect.arrayContaining(['StopPodLogs']))
  })
})
